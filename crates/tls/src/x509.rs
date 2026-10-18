//! X.509 certificate parsing, chain building, signature verification
//! (RSA PKCS#1 v1.5, RSA-PSS, ECDSA P-256/P-384), hostname matching,
//! and loading the OS trust store.

use crate::bignum::Big;
use crate::curve;
use crate::der;
use crate::sha2::{self, HashAlg};

#[derive(Debug)]
pub struct Cert {
    pub raw: Vec<u8>,
    pub tbs: Vec<u8>,
    pub sig_alg: String,
    pub sig: Vec<u8>,
    pub issuer: Vec<u8>,  // DER of issuer Name
    pub subject: Vec<u8>, // DER of subject Name
    pub subject_cn: Option<String>,
    pub san_dns: Vec<String>,
    pub spki_alg: String,
    pub spki: Spki,
    pub not_before: i64,
    pub not_after: i64,
    pub is_ca: bool,
}

#[derive(Debug)]
pub enum Spki {
    Rsa { n: Big, e: Big },
    Ec { curve: EcCurve, x: Big, y: Big },
    Other(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EcCurve {
    P256,
    P384,
}

pub fn parse_cert(data: &[u8]) -> Result<Cert, String> {
    let (top, rest) = der::read_tlv(data)?;
    if !rest.is_empty() || top.tag != 0x30 {
        return Err("cert: bad top".into());
    }
    let mut it = der::children(top.value);
    let tbs = it.next().ok_or("cert: no tbs")?;
    let sig_alg = it.next().ok_or("cert: no sigalg")?;
    let sig = it.next().ok_or("cert: no sig")?;
    if tbs.tag != 0x30 {
        return Err("cert: bad tbs".into());
    }
    let mut tb = der::children(tbs.value);
    let mut cur = tb.next().ok_or("cert: empty tbs")?;
    // optional [0] version
    let mut version = 1;
    if cur.tag == 0xa0 {
        let v = der::children(cur.value).next().ok_or("cert: bad ver")?;
        version = v.value.first().copied().unwrap_or(0) as u32 + 1;
        cur = tb.next().ok_or("cert: no serial")?;
    }
    let _serial = cur.raw;
    let _sig_in_tbs = tb.next().ok_or("cert: no tbs sig")?;
    let issuer = tb.next().ok_or("cert: no issuer")?;
    let validity = tb.next().ok_or("cert: no validity")?;
    let subject = tb.next().ok_or("cert: no subject")?;
    let spki = tb.next().ok_or("cert: no spki")?;

    // validity
    let mut vi = der::children(validity.value);
    let nb = vi.next().ok_or("cert: no nb")?;
    let na = vi.next().ok_or("cert: no na")?;
    let not_before = parse_time(nb.tag, nb.value)?;
    let not_after = parse_time(na.tag, na.value)?;

    // subject CN
    let subject_cn = find_cn(subject.value);

    // SPKI
    let mut si = der::children(spki.value);
    let alg = si.next().ok_or("cert: no spki alg")?;
    let key_bits = si.next().ok_or("cert: no spki bits")?;
    let mut ai = der::children(alg.value);
    let alg_oid = ai.next().map(|t| der::oid_str(t.value)).unwrap_or_default();
    let params = ai.next();
    let spki_parsed = match alg_oid.as_str() {
        "1.2.840.113549.1.1.1" => {
            let key = der::bit_string(key_bits.value)?;
            let mut ki = der::children(key);
            let n = ki.next().ok_or("rsa: no n")?;
            let e = ki.next().ok_or("rsa: no e")?;
            Spki::Rsa {
                n: Big::from_be_bytes(der::int_bytes(n.value)),
                e: Big::from_be_bytes(der::int_bytes(e.value)),
            }
        }
        "1.2.840.10045.2.1" => {
            let curve_oid = params.map(|t| der::oid_str(t.value)).unwrap_or_default();
            let curve = match curve_oid.as_str() {
                "1.2.840.10045.3.1.7" => EcCurve::P256,
                "1.3.132.0.34" => EcCurve::P384,
                _ => return Err(format!("unsupported curve {}", curve_oid)),
            };
            let pt = der::bit_string(key_bits.value)?;
            let coord_len = match curve {
                EcCurve::P256 => 32,
                EcCurve::P384 => 48,
            };
            if pt.len() != 1 + 2 * coord_len || pt[0] != 0x04 {
                return Err("ec: bad point".into());
            }
            Spki::Ec {
                curve,
                x: Big::from_be_bytes(&pt[1..1 + coord_len]),
                y: Big::from_be_bytes(&pt[1 + coord_len..]),
            }
        }
        _ => Spki::Other(spki.value.to_vec()),
    };

    // extensions (version 3): [3] explicit
    let mut san_dns = Vec::new();
    let mut is_ca = false;
    if version == 3 {
        for t in tb {
            if t.tag == 0xa3 {
                if let Some(exts) = der::children(t.value).next() {
                    for ext in der::children(exts.value) {
                        let mut ei = der::children(ext.value);
                        let oid = ei.next().map(|t| der::oid_str(t.value)).unwrap_or_default();
                        let mut nxt = ei.next();
                        // optional critical BOOLEAN
                        if let Some(ref t2) = nxt {
                            if t2.tag == 0x01 {
                                nxt = ei.next();
                            }
                        }
                        let Some(val) = nxt else { continue };
                        if val.tag != 0x04 {
                            continue;
                        }
                        match oid.as_str() {
                            "2.5.29.17" => {
                                // subjectAltName
                                if let Some(seq) = der::children(val.value).next() {
                                    for gn in der::children(seq.value) {
                                        if gn.tag == 0x82 {
                                            san_dns.push(der::to_string(gn.value));
                                        }
                                    }
                                }
                            }
                            "2.5.29.19" => {
                                // basicConstraints: SEQ { cA BOOL }
                                if let Some(seq) = der::children(val.value).next() {
                                    for f in der::children(seq.value) {
                                        if f.tag == 0x01 {
                                            is_ca = f.value.first().copied().unwrap_or(0) != 0;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    let mut sai = der::children(sig_alg.value);
    let sig_alg_oid = sai
        .next()
        .map(|t| der::oid_str(t.value))
        .unwrap_or_default();
    Ok(Cert {
        raw: data.to_vec(),
        tbs: tbs.raw.to_vec(),
        sig_alg: sig_alg_oid,
        sig: der::bit_string(sig.value)?.to_vec(),
        issuer: issuer.raw.to_vec(),
        subject: subject.raw.to_vec(),
        subject_cn,
        san_dns,
        spki_alg: alg_oid,
        spki: spki_parsed,
        not_before,
        not_after,
        is_ca,
    })
}

fn find_cn(name_value: &[u8]) -> Option<String> {
    for rdn in der::children(name_value) {
        if rdn.tag != 0x31 {
            continue;
        }
        for atv in der::children(rdn.value) {
            let mut ai = der::children(atv.value);
            let oid = ai.next().map(|t| der::oid_str(t.value)).unwrap_or_default();
            let val = ai.next();
            if oid == "2.5.4.3" {
                if let Some(v) = val {
                    return Some(der::to_string(v.value));
                }
            }
        }
    }
    None
}

/// UTCTime (0x17) / GeneralizedTime (0x18) → unix seconds.
fn parse_time(tag: u8, v: &[u8]) -> Result<i64, String> {
    let s = der::to_string(v);
    let (y, rest) = match tag {
        0x17 => {
            let yy: i64 = s[0..2].parse().map_err(|_| "bad time")?;
            let y = if yy >= 50 { 1900 + yy } else { 2000 + yy };
            (y, &s[2..])
        }
        0x18 => (s[0..4].parse().map_err(|_| "bad time")?, &s[4..]),
        _ => return Err("bad time tag".into()),
    };
    let mo: i64 = rest[0..2].parse().map_err(|_| "bad time")?;
    let d: i64 = rest[2..4].parse().map_err(|_| "bad time")?;
    let h: i64 = rest[4..6].parse().map_err(|_| "bad time")?;
    let mi: i64 = rest[6..8].parse().map_err(|_| "bad time")?;
    let sec: i64 = rest.get(8..10).unwrap_or("00").parse().unwrap_or(0);
    Ok(days_from_civil(y, mo as u32, d as u32) * 86400 + h * 3600 + mi * 60 + sec)
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    // Howard Hinnant's algorithm
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

// ---------------- signature verification ----------------

fn mgf1(alg: &HashAlg, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ctr = 0u32;
    while out.len() < len {
        let mut m = seed.to_vec();
        m.extend_from_slice(&ctr.to_be_bytes());
        out.extend_from_slice(&alg.hash(&m));
        ctr += 1;
    }
    out.truncate(len);
    out
}

fn rsa_public(spki_n: &Big, spki_e: &Big, sig: &[u8]) -> Vec<u8> {
    let s = Big::from_be_bytes(sig);
    let em = s.modpow(spki_e, spki_n);
    let k = spki_n.bits().div_ceil(8);
    em.to_be_bytes(k)
}

fn hash_for(alg_id: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match alg_id {
        "sha256" => Ok(sha2::sha256(data).to_vec()),
        "sha384" => Ok(sha2::sha384(data).to_vec()),
        "sha512" => Ok(sha2::sha512(data).to_vec()),
        _ => Err(format!("hash {}", alg_id)),
    }
}

/// Map signature-algorithm OID → (kind, hash, curve_hint).
fn sig_kind(oid: &str) -> Result<(&'static str, &'static str), String> {
    match oid {
        "1.2.840.113549.1.1.11" => Ok(("rsa_pkcs1", "sha256")),
        "1.2.840.113549.1.1.12" => Ok(("rsa_pkcs1", "sha384")),
        "1.2.840.113549.1.1.13" => Ok(("rsa_pkcs1", "sha512")),
        "1.2.840.113549.1.1.10" => Ok(("rsa_pss", "sha256")), // params parsed below TODO default
        "1.2.840.10045.4.3.2" => Ok(("ecdsa", "sha256")),
        "1.2.840.10045.4.3.3" => Ok(("ecdsa", "sha384")),
        "1.2.840.10045.4.3.4" => Ok(("ecdsa", "sha512")),
        _ => Err(format!("unsupported signature alg {}", oid)),
    }
}

/// Verify `sig` over `msg` with the issuer's public key.
pub fn verify_signature(
    sig_alg_oid: &str,
    msg: &[u8],
    sig: &[u8],
    spki: &Spki,
) -> Result<bool, String> {
    let (kind, hash_name) = sig_kind(sig_alg_oid)?;
    let hash_alg = match hash_name {
        "sha256" => HashAlg::Sha256,
        "sha384" | "sha512" => HashAlg::Sha384, // 512 used rarely; PSS handled below
        _ => HashAlg::Sha256,
    };
    let digest = hash_for(hash_name, msg)?;
    match (kind, spki) {
        ("rsa_pkcs1", Spki::Rsa { n, e }) => {
            let em = rsa_public(n, e, sig);
            let k = em.len();
            // EM = 0x00 0x01 FF* 0x00 || DigestInfo
            if k < 11 + digest.len() + 15 || em[0] != 0x00 || em[1] != 0x01 {
                return Ok(false);
            }
            let mut i = 2;
            while i < k && em[i] == 0xff {
                i += 1;
            }
            if i >= k || em[i] != 0x00 || i < 10 {
                return Ok(false);
            }
            let t = &em[i + 1..];
            let prefix: &[u8] = match hash_name {
                "sha256" => &[
                    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04,
                    0x02, 0x01, 0x05, 0x00, 0x04, 0x20,
                ],
                "sha384" => &[
                    0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04,
                    0x02, 0x02, 0x05, 0x00, 0x04, 0x30,
                ],
                "sha512" => &[
                    0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04,
                    0x02, 0x03, 0x05, 0x00, 0x04, 0x40,
                ],
                _ => return Ok(false),
            };
            Ok(t.len() == prefix.len() + digest.len()
                && t[..prefix.len()] == *prefix
                && t[prefix.len()..] == digest[..])
        }
        ("rsa_pss", Spki::Rsa { n, e }) => {
            // TLS always signals pss hash via the SignatureScheme; in cert
            // chains we try the three SHA-2 variants in order.
            for hn in ["sha256", "sha384", "sha512"] {
                let alg = match hn {
                    "sha256" => HashAlg::Sha256,
                    "sha384" => HashAlg::Sha384,
                    _ => HashAlg::Sha512,
                };
                let dg = hash_for(hn, msg)?;
                if pss_verify(n, e, sig, &alg, &dg) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        ("ecdsa", Spki::Ec { curve, x, y }) => {
            // sig = SEQ { r, s }
            let (seq, _) = der::read_tlv(sig).map_err(|e| e)?;
            if seq.tag != 0x30 {
                return Ok(false);
            }
            let mut it = der::children(seq.value);
            let r = it.next().ok_or("ecdsa: no r")?;
            let s = it.next().ok_or("ecdsa: no s")?;
            let params = match curve {
                EcCurve::P256 => curve::p256(),
                EcCurve::P384 => curve::p384(),
            };
            let _ = hash_alg;
            Ok(curve::ecdsa_verify(
                &params,
                x,
                y,
                &digest,
                &Big::from_be_bytes(der::int_bytes(r.value)),
                &Big::from_be_bytes(der::int_bytes(s.value)),
            ))
        }
        _ => Err("key/sig type mismatch".into()),
    }
}

/// EMSA-PSS verify with salt length = hash length (TLS convention).
fn pss_verify(n: &Big, e: &Big, sig: &[u8], alg: &HashAlg, mhash: &[u8]) -> bool {
    let k = n.bits().div_ceil(8);
    let em = rsa_public(n, e, sig);
    if em.len() != k || em[k - 1] != 0xbc {
        return false;
    }
    let hlen = alg.out_len();
    if k < hlen + hlen + 2 {
        return false;
    }
    let db_len = k - hlen - 1;
    let (masked_db, h) = em.split_at(db_len);
    let mask = mgf1(alg, h, db_len);
    let mut db = masked_db.to_vec();
    for i in 0..db_len {
        db[i] ^= mask[i];
    }
    // clear leftmost bits per emBits = 8k - (modulus_bits-1)... For RSA
    // modulus bit length b, emLen = ceil((b-1)/8); if emLen == k, top bits
    // of DB[0] must be zeroed: unused = 8k - (b-1)
    let em_bits = n.bits() - 1;
    let unused = 8 * k - em_bits;
    if unused > 0 {
        db[0] &= 0xff >> unused;
    }
    // DB = PS 0x00* || 0x01 || salt
    let mut i = 0;
    while i < db_len && db[i] == 0 {
        i += 1;
    }
    if i >= db_len || db[i] != 0x01 {
        return false;
    }
    let salt = &db[i + 1..];
    let mut m = vec![0u8; 8];
    m.extend_from_slice(mhash);
    m.extend_from_slice(salt);
    let expect = alg.hash(&m);
    &expect[..] == h
}

// ---------------- chain & hostname ----------------

pub struct TrustStore {
    pub roots: Vec<Cert>,
}

impl TrustStore {
    pub fn load_system() -> TrustStore {
        let mut roots = Vec::new();
        let mut files = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/etc/ssl/certs") {
            for e in rd.flatten() {
                files.push(e.path());
            }
        }
        for f in [
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
        ] {
            files.push(std::path::PathBuf::from(f));
        }
        let mut seen = std::collections::HashSet::new();
        for f in files {
            let Ok(data) = std::fs::read(&f) else {
                continue;
            };
            for pem in pem_blocks(&data) {
                if seen.insert(pem.len()) {
                    if let Ok(c) = parse_cert(&pem) {
                        roots.push(c);
                    }
                }
            }
        }
        TrustStore { roots }
    }
}

/// Extract DER cert blocks from PEM data.
pub fn pem_blocks(data: &[u8]) -> Vec<Vec<u8>> {
    let s = String::from_utf8_lossy(data);
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for line in s.lines() {
        let l = line.trim();
        if l == "-----BEGIN CERTIFICATE-----" {
            cur = Some(String::new());
        } else if l == "-----END CERTIFICATE-----" {
            if let Some(b) = cur.take() {
                if let Ok(d) = sol_b64(&b) {
                    out.push(d);
                }
            }
        } else if let Some(b) = cur.as_mut() {
            b.push_str(l);
        }
    }
    out
}

fn sol_b64(s: &str) -> Result<Vec<u8>, String> {
    // Local copy to avoid net->tls dependency cycle.
    fn v(c: u8) -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err("b64".into()),
        }
    }
    let b: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    for ch in b.chunks(4) {
        if ch.len() < 2 {
            break;
        }
        let vals: Vec<u32> = ch
            .iter()
            .map(|&c| if c == b'=' { Ok(0) } else { v(c) })
            .collect::<Result<_, _>>()?;
        let x = (vals[0] << 18)
            | (vals[1] << 12)
            | (vals.get(2).copied().unwrap_or(0) << 6)
            | vals.get(3).copied().unwrap_or(0);
        out.push((x >> 16) as u8);
        if ch.len() > 2 && ch[2] != b'=' {
            out.push((x >> 8) as u8);
        }
        if ch.len() > 3 && ch[3] != b'=' {
            out.push(x as u8);
        }
    }
    Ok(out)
}

/// Verify the peer chain: leaf + presented intermediates → trusted root.
/// `now` is unix seconds.
pub fn verify_chain(
    leaf: &Cert,
    chain: &[Cert],
    store: &TrustStore,
    hostname: &str,
    now: i64,
) -> Result<(), String> {
    if now < leaf.not_before || now > leaf.not_after {
        return Err("certificate expired".into());
    }
    if !hostname_matches(leaf, hostname) {
        return Err(format!(
            "hostname '{}' does not match certificate",
            hostname
        ));
    }
    // Build path: find issuer of leaf among presented chain or roots.
    let mut cur: &Cert = leaf;
    let mut depth = 0;
    let mut used = vec![false; chain.len()];
    loop {
        if depth > 8 {
            return Err("chain too long".into());
        }
        // self-signed root?
        if let Some(root) = store
            .roots
            .iter()
            .find(|r| r.subject == cur.issuer && verify_sig_ok(cur, r))
        {
            let _ = root;
            return Ok(());
        }
        // next intermediate
        let mut found = None;
        for (i, c) in chain.iter().enumerate() {
            if !used[i] && c.subject == cur.issuer && verify_sig_ok(cur, c) {
                if !c.is_ca {
                    return Err("intermediate is not a CA".into());
                }
                used[i] = true;
                found = Some(c);
                break;
            }
        }
        match found {
            Some(c) => {
                if now < c.not_before || now > c.not_after {
                    return Err("intermediate expired".into());
                }
                cur = c;
                depth += 1;
            }
            None => return Err("no path to trusted root".into()),
        }
    }
}

fn verify_sig_ok(child: &Cert, issuer: &Cert) -> bool {
    verify_signature(&child.sig_alg, &child.tbs, &child.sig, &issuer.spki).unwrap_or(false)
}

fn hostname_matches(cert: &Cert, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let names: Vec<&String> = if !cert.san_dns.is_empty() {
        cert.san_dns.iter().collect()
    } else {
        cert.subject_cn.iter().collect()
    };
    for n in names {
        let pat = n.to_ascii_lowercase();
        if pat == host {
            return true;
        }
        if let Some(suf) = pat.strip_prefix("*.") {
            // wildcard covers one label
            if host.ends_with(suf)
                && host[..host.len() - suf.len()]
                    .trim_end_matches('.')
                    .find('.')
                    .is_none()
                && host.len() > suf.len() + 1
            {
                return true;
            }
        }
    }
    false
}
