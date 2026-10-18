//! Bounded shared HTTP freshness cache and ordered parallel GET batches.
use crate::{Client, Request, Response};
use sol_url::Url;
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;
use std::time::Instant;

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const BATCH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    pub entries: usize,
    pub bytes: usize,
}
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) struct CacheKey {
    url: String,
    verify_tls: bool,
    user_agent: String,
    headers: Vec<(String, String)>,
}
struct Entry {
    response: Response,
    expires: Instant,
    stored: Instant,
    age: u64,
}
#[derive(Default)]
pub(super) struct ResourceCache {
    entries: HashMap<CacheKey, Entry>,
    order: VecDeque<CacheKey>,
    stats: CacheStats,
}
impl ResourceCache {
    fn remove(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.stats.bytes -= entry.response.body.len();
        }
        self.order.retain(|item| item != key);
        self.stats.entries = self.entries.len();
    }
}

impl Client {
    pub fn clear_cache(&mut self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = ResourceCache::default();
    }
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).stats
    }
    pub(super) fn cache_key(&self, request: &Request) -> Option<CacheKey> {
        // This cache has no per-user partition or validator revalidation path.
        // Requests carrying state, credentials, or conditional/range semantics
        // therefore cannot safely share the simple fresh-body lookup below.
        if request.method != "GET"
            || !request.body.is_empty()
            || !matches!(request.url.scheme.as_str(), "http" | "https")
            || self.cookies.header_for(&request.url).is_some()
        {
            return None;
        }
        let mut headers: Vec<_> = request
            .headers
            .iter()
            .chain(&self.extra_headers)
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
            .collect();
        if headers.iter().any(|(name, _)| {
            matches!(
                name.as_str(),
                "authorization"
                    | "proxy-authorization"
                    | "cookie"
                    | "range"
                    | "cache-control"
                    | "if-none-match"
                    | "if-modified-since"
            )
        }) {
            return None;
        }
        headers.sort();
        let mut url = request.url.clone();
        url.fragment = None;
        Some(CacheKey {
            url: url.serialize(),
            verify_tls: self.verify_tls,
            user_agent: self.user_agent.clone(),
            headers,
        })
    }
    pub(super) fn cached(&self, key: &CacheKey) -> Option<Response> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let fresh = cache.entries.get(key).is_some_and(|entry| {
            entry.expires > Instant::now() && entry.response.body.len() <= self.max_body_bytes
        });
        if !fresh {
            cache.remove(key);
            cache.stats.misses += 1;
            return None;
        }
        cache.stats.hits += 1;
        cache.order.retain(|item| item != key);
        cache.order.push_back(key.clone());
        let entry = &cache.entries[key];
        let mut response = entry.response.clone();
        response
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("age"));
        response.headers.push((
            "age".into(),
            entry
                .age
                .saturating_add(entry.stored.elapsed().as_secs())
                .to_string(),
        ));
        Some(response)
    }
    pub(super) fn remember(&self, key: CacheKey, response: &Response, elapsed: Duration) {
        if response.status != 200
            || response.body.len() > CACHE_BYTES
            || response.header("set-cookie").is_some()
            || response
                .headers_all("vary")
                .iter()
                .any(|value| value.split(',').any(|name| name.trim() == "*"))
        {
            return;
        }
        let mut max_age = None;
        for value in response.headers_all("cache-control") {
            for directive in value.split(',').map(str::trim) {
                let (name, value) = directive.split_once('=').unwrap_or((directive, ""));
                if ["no-store", "no-cache", "private"]
                    .iter()
                    .any(|v| name.eq_ignore_ascii_case(v))
                {
                    return;
                }
                if name.eq_ignore_ascii_case("max-age") {
                    let Ok(seconds) = value.trim_matches('"').parse::<u64>() else {
                        return;
                    };
                    max_age = Some(max_age.map_or(seconds, |previous: u64| previous.min(seconds)));
                }
            }
        }
        let now = std::time::SystemTime::now();
        let date = response
            .header("date")
            .and_then(|value| httpdate::parse_http_date(value).ok());
        let apparent_age = date
            .and_then(|date| now.duration_since(date).ok())
            .unwrap_or_default();
        let age = response
            .header("age")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let age = apparent_age.max(Duration::from_secs(age).saturating_add(elapsed));
        let lifetime = max_age
            .map(Duration::from_secs)
            .or_else(|| {
                let expires = httpdate::parse_http_date(response.header("expires")?).ok()?;
                expires.duration_since(date.unwrap_or(now)).ok()
            })
            .unwrap_or_default();
        // A proxy may deliver a response that is already old. Restarting its
        // full freshness lifetime locally would serve it beyond the origin's
        // deadline; the cap also limits retention of overly generous lifetimes.
        let remaining = lifetime.saturating_sub(age).min(Duration::from_secs(3600));
        if remaining.is_zero() {
            return;
        }
        let age = age.as_secs();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.remove(&key);
        while cache.stats.bytes + response.body.len() > CACHE_BYTES
            || cache.entries.len() >= CACHE_ENTRIES
        {
            if let Some(oldest) = cache.order.front().cloned() {
                cache.remove(&oldest);
            } else {
                break;
            }
        }
        cache.stats.bytes += response.body.len();
        cache.order.push_back(key.clone());
        cache.entries.insert(
            key,
            Entry {
                response: response.clone(),
                expires: Instant::now() + remaining,
                stored: Instant::now(),
                age,
            },
        );
        cache.stats.entries = cache.entries.len();
    }
    pub(super) fn invalidate(&self, url: &Url) {
        let mut url = url.clone();
        url.fragment = None;
        let url = url.serialize();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<_> = cache
            .entries
            .keys()
            .filter(|key| key.url == url)
            .cloned()
            .collect();
        for key in keys {
            cache.remove(&key);
        }
    }

    /// Fetch independent resources concurrently, deduplicating URLs and keeping
    /// results in input order. At most eight workers and 64 MiB retained bodies.
    /// Cookie effects are committed in input order after the batch completes.
    pub fn get_many(&mut self, urls: &[Url], parallelism: usize) -> Vec<Result<Response, String>> {
        let workers = parallelism.clamp(1, 8).min(urls.len());
        if workers == 0 {
            return Vec::new();
        }
        let mut unique = Vec::new();
        let mut lookup = HashMap::new();
        let mut mapping = Vec::new();
        for url in urls {
            let mut normalized = url.clone();
            normalized.fragment = None;
            let index = *lookup.entry(normalized.serialize()).or_insert_with(|| {
                unique.push(normalized);
                unique.len() - 1
            });
            mapping.push(index);
        }
        // Agent construction must precede cloning: otherwise each worker could
        // establish an independent pool and defeat connection reuse.
        self.agent();
        let template = self.clone();
        let next = AtomicUsize::new(0);
        // A bounded channel provides backpressure while the receiver applies
        // the retained-body budget. A fast set of workers must not build an
        // unbounded queue of completed response bodies.
        let (sender, receiver) = mpsc::sync_channel(workers);
        let mut results: Vec<_> = (0..unique.len()).map(|_| None).collect();
        let mut retained = 0usize;
        std::thread::scope(|scope| {
            for _ in 0..workers.min(unique.len()) {
                let sender = sender.clone();
                let template = &template;
                let urls = &unique;
                let next = &next;
                scope.spawn(move || loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(url) = urls.get(index) else {
                        break;
                    };
                    let mut client = template.clone();
                    client.cookie_events = Some(Vec::new());
                    let result = client.get(url);
                    let events = client.cookie_events.take().unwrap_or_default();
                    if sender.send((index, result, events)).is_err() {
                        break;
                    }
                });
            }
            drop(sender);
            for (index, mut result, events) in receiver {
                if let Ok(response) = &result {
                    if response.body.len() > BATCH_BYTES - retained {
                        result = Err("subresource batch memory limit exceeded".into());
                    } else {
                        retained += response.body.len();
                    }
                }
                results[index] = Some((result, events));
            }
        });
        // Requests use independent cookie snapshots while running concurrently.
        // Replaying changes in input order avoids letting timing decide which
        // Set-Cookie value wins when several resources update the same cookie.
        let mut unique_results = Vec::with_capacity(unique.len());
        for result in results {
            let (result, events) =
                result.unwrap_or_else(|| (Err("resource worker failed".into()), Vec::new()));
            for (url, value) in events {
                self.cookies.set(&value, &url);
            }
            unique_results.push(result);
        }
        // Duplicate consumers still need owned bodies. Charging those clones
        // against the same budget avoids defeating the batch limit with many
        // references to one large URL; the last consumer can reuse its storage.
        let mut uses = vec![0usize; unique_results.len()];
        for &index in &mapping {
            uses[index] += 1;
        }
        let mut output = Vec::with_capacity(urls.len());
        for index in mapping {
            uses[index] -= 1;
            if uses[index] == 0 {
                output.push(std::mem::replace(
                    &mut unique_results[index],
                    Err("consumed".into()),
                ));
            } else {
                if let Ok(response) = &unique_results[index] {
                    if response.body.len() > BATCH_BYTES - retained {
                        output.push(Err("subresource batch memory limit exceeded".into()));
                        continue;
                    }
                    retained += response.body.len();
                }
                output.push(unique_results[index].clone());
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(index: usize) -> Request {
        Request {
            method: "GET".into(),
            url: Url::parse(&format!("http://cache.invalid/{index}")).unwrap(),
            headers: vec![],
            body: vec![],
        }
    }
    fn response(index: usize, bytes: usize) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            headers: vec![("cache-control".into(), "max-age=60".into())],
            body: vec![0; bytes],
            url: request(index).url,
        }
    }
    #[test]
    fn cache_evicts_lru_entries_and_expired_responses() {
        let client = Client::new();
        for index in 0..CACHE_ENTRIES {
            client.remember(
                client.cache_key(&request(index)).unwrap(),
                &response(index, 1),
                Duration::ZERO,
            );
        }
        let first = client.cache_key(&request(0)).unwrap();
        assert!(client.cached(&first).is_some());
        client.remember(
            client.cache_key(&request(CACHE_ENTRIES)).unwrap(),
            &response(CACHE_ENTRIES, 1),
            Duration::ZERO,
        );
        assert!(client
            .cached(&client.cache_key(&request(1)).unwrap())
            .is_none());
        assert!(client.cached(&first).is_some());
        assert_eq!(client.cache_stats().entries, CACHE_ENTRIES);
        client
            .cache
            .lock()
            .unwrap()
            .entries
            .get_mut(&first)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(client.cached(&first).is_none());
        assert_eq!(client.cache_stats().entries, CACHE_ENTRIES - 1);
    }
    #[test]
    fn cache_caps_retained_body_memory() {
        let client = Client::new();
        let large = response(0, CACHE_BYTES / 2 + 1);
        client.remember(
            client.cache_key(&request(0)).unwrap(),
            &large,
            Duration::ZERO,
        );
        client.remember(
            client.cache_key(&request(1)).unwrap(),
            &large,
            Duration::ZERO,
        );
        assert_eq!(client.cache_stats().entries, 1);
        assert!(client.cache_stats().bytes <= CACHE_BYTES);
    }
}
