//! TLS 1.3 client handshake and record layer (RFC 8446).
//!
//! - X25519 key exchange
//! - AES-128/256-GCM and ChaCha20-Poly1305 cipher suites
//! - Full X.509 chain + hostname verification (via `x509`)
//! - HelloRetryRequest (cookie echo), middlebox-compat CCS filtering
//! - Post-handshake NewSessionTicket (ignored) and KeyUpdate

use crate::aead::{make_nonce, Suite};
use crate::curve;
use crate::rng;
use crate::sha2::HashAlg;
use crate::x509;
use std::io::{self, Cursor, Read, Write};
use std::net::TcpStream;

const REC_CCS: u8 = 20;
const REC_ALERT: u8 = 21;
const REC_HS: u8 = 22;
const REC_APP: u8 = 23;

const HRR_RANDOM: [u8; 32] = [
    0xcf, 0x21, 0xad, 0x74, 0xe5, 0x9a, 0x61, 0x11, 0xbe, 0x1d, 0x8c, 0x02, 0x1e, 0x65, 0xb8, 0x91,
    0xc2, 0xa2, 0x11, 0x16, 0x7a, 0xbb, 0x8c, 0x5e, 0x07, 0x9e, 0x09, 0xe2, 0xc8, 0xa8, 0x33, 0x9c,
];

pub struct TlsStream {
    tcp: TcpStream,
    suite: Suite,
    hash: HashAlg,
    c_secret: Vec<u8>,
    s_secret: Vec<u8>,
    c_key: Vec<u8>,
    c_iv: [u8; 12],
    s_key: Vec<u8>,
    s_iv: [u8; 12],
    c_seq: u64,
    s_seq: u64,
    plain: Cursor<Vec<u8>>, // buffered decrypted app data
    closed: bool,
}

struct Shs {
    /// decrypted handshake bytes queue (records may coalesce/split msgs)
    hs: Vec<u8>,
    /// next expected read seq for encrypted records
    seq: u64,
}

impl TlsStream {
    pub fn connect(tcp: TcpStream, hostname: &str, verify: bool) -> Result<TlsStream, String> {
        handshake(&tcp, hostname, verify)
    }
}

impl Read for TlsStream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.plain.position() == self.plain.get_ref().len() as u64 && !self.closed {
            self.fill().map_err(io::Error::other)?;
        }
        self.plain.read(out)
    }
}

impl Write for TlsStream {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.is_empty() {
            return Ok(0);
        }
        // chunk to <= 2^14 plaintext
        let max = 16384.min(data.len());
        self.send_app(&data[..max]).map_err(std::io::Error::other)?;
        Ok(max)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.tcp.flush()
    }
}

impl TlsStream {
    fn send_app(&mut self, data: &[u8]) -> Result<(), String> {
        let mut inner = data.to_vec();
        inner.push(REC_APP);
        let aad = hdr(REC_APP, inner.len() + 16);
        let sealed = self.suite.seal(
            &self.c_key,
            &make_nonce(&self.c_iv, self.c_seq),
            &aad,
            &inner,
        );
        self.c_seq += 1;
        let mut rec = aad;
        rec.extend_from_slice(&sealed);
        self.tcp.write_all(&rec).map_err(|e| e.to_string())
    }

    fn fill(&mut self) -> Result<(), String> {
        // Read until we get application data; handle post-handshake msgs.
        loop {
            let (ty, body) = self.read_record()?;
            match ty {
                REC_APP => {}
                _ => return Err("unexpected plaintext record".into()),
            }
            // inner content type = last byte
            if body.is_empty() {
                continue;
            }
            let inner_ty = body[body.len() - 1];
            let content = &body[..body.len() - 1];
            match inner_ty {
                REC_APP => {
                    if content.is_empty() {
                        continue;
                    }
                    self.plain = Cursor::new(content.to_vec());
                    return Ok(());
                }
                REC_HS => {
                    self.handle_post_hs(content)?;
                }
                REC_ALERT => {
                    self.closed = true;
                    if content.first() == Some(&1) {
                        return Ok(()); // close_notify (level byte may precede)
                    }
                    return Err(format!("tls alert: {:?}", content));
                }
                REC_CCS => {}
                _ => {}
            }
        }
    }

    fn handle_post_hs(&mut self, content: &[u8]) -> Result<(), String> {
        if content.len() < 4 {
            return Ok(());
        }
        let ty = content[0];
        let len = u32::from_be_bytes([0, content[1], content[2], content[3]]) as usize;
        let body = &content[4..4 + len.min(content.len() - 4)];
        match ty {
            4 => {} // NewSessionTicket — no resumption, ignore
            24 => {
                // KeyUpdate
                self.s_secret =
                    self.hash
                        .expand_label(&self.s_secret, "traffic upd", &[], self.hash.out_len());
                self.s_key =
                    self.hash
                        .expand_label(&self.s_secret, "key", &[], self.suite.key_len());
                self.s_iv
                    .copy_from_slice(&self.hash.expand_label(&self.s_secret, "iv", &[], 12));
                self.s_seq = 0;
                if body.first() == Some(&1) {
                    // update_requested: reply with our own KeyUpdate(0)
                    self.c_secret = self.hash.expand_label(
                        &self.c_secret,
                        "traffic upd",
                        &[],
                        self.hash.out_len(),
                    );
                    self.c_key =
                        self.hash
                            .expand_label(&self.c_secret, "key", &[], self.suite.key_len());
                    self.c_iv.copy_from_slice(&self.hash.expand_label(
                        &self.c_secret,
                        "iv",
                        &[],
                        12,
                    ));
                    self.c_seq = 0;
                    let mut msg = vec![24u8, 0, 0, 1, 0];
                    msg.push(REC_HS);
                    let aad = hdr(REC_APP, msg.len() + 16);
                    let sealed = self.suite.seal(
                        &self.c_key,
                        &make_nonce(&self.c_iv, self.c_seq),
                        &aad,
                        &msg,
                    );
                    self.c_seq += 1;
                    let mut rec = aad;
                    rec.extend_from_slice(&sealed);
                    self.tcp.write_all(&rec).map_err(|e| e.to_string())?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Read and decrypt one protected record.
    fn read_record(&mut self) -> Result<(u8, Vec<u8>), String> {
        loop {
            let (ty, raw) = read_raw_record(&mut self.tcp)?;
            match ty {
                REC_CCS => continue,
                REC_APP => {
                    let mut aad = vec![ty, 0x03, 0x03];
                    aad.extend_from_slice(&(raw.len() as u16).to_be_bytes());
                    let pt = self.suite.open(
                        &self.s_key,
                        &make_nonce(&self.s_iv, self.s_seq),
                        &aad,
                        &raw,
                    );
                    self.s_seq += 1;
                    return Ok((REC_APP, pt?));
                }
                REC_ALERT => return Err(alert_str(&raw)),
                t2 => return Err(format!("unexpected record type {}", t2)),
            }
        }
    }
}

// ---------------- handshake ----------------

fn hdr(ty: u8, len: usize) -> Vec<u8> {
    vec![ty, 0x03, 0x03, (len >> 8) as u8, len as u8]
}

fn record(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut r = hdr(ty, body.len());
    r.extend_from_slice(body);
    r
}

/// Read one TLS record (plaintext). Returns (type, body).
fn read_raw_record<R: Read>(reader: &mut R) -> Result<(u8, Vec<u8>), String> {
    let mut h = [0; 5];
    reader.read_exact(&mut h).map_err(|e| e.to_string())?;
    let ty = h[0];
    let len = u16::from_be_bytes([h[3], h[4]]) as usize;
    if len > 18432 {
        return Err("record too large".into());
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok((ty, body))
}

fn alert_str(body: &[u8]) -> String {
    if body.len() >= 2 {
        format!("TLS alert {} (level {})", body[1], body[0])
    } else {
        "TLS alert".into()
    }
}

struct Reader<'a> {
    d: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(d: &'a [u8]) -> Reader<'a> {
        Reader { d, pos: 0 }
    }
    fn u8(&mut self) -> Result<u8, String> {
        if self.pos >= self.d.len() {
            return Err("hs: eof".into());
        }
        let v = self.d[self.pos];
        self.pos += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn u24(&mut self) -> Result<u32, String> {
        Ok((self.u16()? as u32) << 8 | self.u8()? as u32)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.d.len() {
            return Err("hs: eof".into());
        }
        let r = &self.d[self.pos..self.pos + n];
        self.pos += n;
        Ok(r)
    }
    fn vec8(&mut self) -> Result<&'a [u8], String> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn vec16(&mut self) -> Result<&'a [u8], String> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn vec24(&mut self) -> Result<&'a [u8], String> {
        let n = self.u24()? as usize;
        self.take(n)
    }
    fn done(&self) -> bool {
        self.pos >= self.d.len()
    }
}

struct H {
    tcp: TcpStream,
    host: String,
    verify: bool,
    th: Vec<u8>, // handshake transcript bytes
    hash: HashAlg,
    suite: Suite,
    priv_key: [u8; 32],
    shs: Shs, // server->client encrypted-hs reader state
    c_hs_seq: u64,
    c_hs_key: Vec<u8>,
    c_hs_iv: [u8; 12],
    s_hs_key: Vec<u8>,
    s_hs_iv: [u8; 12],
    hs_secret: Vec<u8>,
    fin_c: Vec<u8>,
    fin_s: Vec<u8>,
    store: Option<x509::TrustStore>,
    /// server's x25519 share (from SH key_share ext)
    shared: Option<Vec<u8>>,
    /// cookie echoed by HRR
    cookie: Option<Vec<u8>>,
}

fn handshake(tcp: &TcpStream, host: &str, verify: bool) -> Result<TlsStream, String> {
    let mut h = H {
        tcp: tcp.try_clone().map_err(|e| e.to_string())?,
        host: host.to_string(),
        verify,
        th: Vec::new(),
        hash: HashAlg::Sha256,
        suite: Suite::Aes128GcmSha256,
        priv_key: rng::rand_array(),
        shs: Shs {
            hs: Vec::new(),
            seq: 0,
        },
        c_hs_seq: 0,
        c_hs_key: vec![],
        c_hs_iv: [0; 12],
        s_hs_key: vec![],
        s_hs_iv: [0; 12],
        hs_secret: vec![],
        fin_c: vec![],
        fin_s: vec![],
        store: None,
        shared: None,
        cookie: None,
    };

    // ---- ClientHello (+HRR retry) ----
    let mut cookie: Option<Vec<u8>> = None;
    for attempt in 0..2 {
        let ch = build_client_hello(&h.host, &h.priv_key, cookie.as_deref());
        h.tcp
            .write_all(&record(REC_HS, &ch))
            .map_err(|e| e.to_string())?;
        if attempt == 0 {
            h.th = ch;
        } else {
            h.th.extend_from_slice(&ch);
        }
        let (sh_msg, is_hrr) = h.read_sh()?;
        if is_hrr {
            if attempt == 1 {
                return Err("double HRR".into());
            }
            // RFC 8446 §4.4.1: replace CH1 with message_hash(ClientHello1)
            let ch1 = std::mem::take(&mut h.th);
            let mh = h.hash.hash(&ch1);
            h.th = vec![254, 0, 0, mh.len() as u8];
            h.th.extend_from_slice(&mh);
            h.th.extend_from_slice(&sh_msg);
            cookie = h.take_cookie();
            continue;
        }
        h.th.extend_from_slice(&sh_msg);
        break;
    }

    // ---- key schedule: handshake secrets ----
    let shared = h.take_shared()?;
    let z = vec![0u8; h.hash.out_len()];
    let early = h.hash.hkdf_extract(&z, &z);
    let empty_hash = h.hash.hash(&[]);
    let derived = h.hash.derive_secret(&early, "derived", &empty_hash);
    h.hs_secret = h.hash.hkdf_extract(&derived, &shared);
    let th = h.hash.hash(&h.th);
    let c_hs = h.hash.derive_secret(&h.hs_secret, "c hs traffic", &th);
    let s_hs = h.hash.derive_secret(&h.hs_secret, "s hs traffic", &th);
    h.c_hs_key = h.hash.expand_label(&c_hs, "key", &[], h.suite.key_len());
    h.c_hs_iv
        .copy_from_slice(&h.hash.expand_label(&c_hs, "iv", &[], 12));
    h.s_hs_key = h.hash.expand_label(&s_hs, "key", &[], h.suite.key_len());
    h.s_hs_iv
        .copy_from_slice(&h.hash.expand_label(&s_hs, "iv", &[], 12));
    h.fin_c = h
        .hash
        .expand_label(&c_hs, "finished", &[], h.hash.out_len());
    h.fin_s = h
        .hash
        .expand_label(&s_hs, "finished", &[], h.hash.out_len());

    // ---- encrypted handshake flight ----
    let mut leaf_der: Option<Vec<u8>> = None;
    let mut chain_der: Vec<Vec<u8>> = Vec::new();
    let mut cert_req_ctx: Option<Vec<u8>> = None;
    loop {
        let (ty, body) = h.read_hs_msg()?;
        let mut raw = vec![ty];
        raw.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        raw.extend_from_slice(&body);
        match ty {
            8 => {} // EncryptedExtensions
            11 => {
                let mut r = Reader::new(&body);
                let _ctx = r.vec8()?;
                let list = r.vec16()?;
                let mut lr = Reader::new(list);
                while !lr.done() {
                    let cert = lr.vec24()?;
                    let _ext = lr.vec16()?;
                    if leaf_der.is_none() {
                        leaf_der = Some(cert.to_vec());
                    } else {
                        chain_der.push(cert.to_vec());
                    }
                }
            }
            13 => {
                let mut r = Reader::new(&body);
                cert_req_ctx = Some(r.vec8()?.to_vec());
            }
            15 => {
                let mut r = Reader::new(&body);
                let scheme = r.u16()?;
                let sig = r.vec16()?.to_vec();
                if h.verify {
                    let leaf = leaf_der.as_ref().ok_or("CV before cert")?;
                    let cert = x509::parse_cert(leaf)?;
                    let thh = h.hash.hash(&h.th);
                    let mut m = vec![0x20u8; 64];
                    m.extend_from_slice(b"TLS 1.3, server CertificateVerify");
                    m.push(0);
                    m.extend_from_slice(&thh);
                    if !verify_cv(scheme, &m, &sig, &cert)? {
                        return Err("CertificateVerify signature invalid".into());
                    }
                }
            }
            20 => {
                if h.verify {
                    let expect = h.hash.hmac(&h.fin_s, &h.hash.hash(&h.th));
                    if expect != body {
                        return Err("bad server Finished".into());
                    }
                }
                h.th.extend_from_slice(&raw);
                let th = h.hash.hash(&h.th);

                if h.verify {
                    let leaf = x509::parse_cert(leaf_der.as_ref().ok_or("no server cert")?)?;
                    let mut chain = Vec::new();
                    for d in &chain_der {
                        chain.push(x509::parse_cert(d)?);
                    }
                    let store = h.store.get_or_insert_with(x509::TrustStore::load_system);
                    x509::verify_chain(
                        &leaf,
                        &chain,
                        store,
                        &h.host,
                        (sol_util::now_ms() / 1000) as i64,
                    )?;
                }

                let derived = h.hash.derive_secret(&h.hs_secret, "derived", &empty_hash);
                let master = h.hash.hkdf_extract(&derived, &z);
                let c_ap = h.hash.derive_secret(&master, "c ap traffic", &th);
                let s_ap = h.hash.derive_secret(&master, "s ap traffic", &th);

                // optional empty client Certificate
                if let Some(ctx) = &cert_req_ctx {
                    let mut cb = Vec::new();
                    cb.push(ctx.len() as u8);
                    cb.extend_from_slice(ctx);
                    cb.extend_from_slice(&[0, 0]);
                    let mut msg = vec![11u8];
                    msg.extend_from_slice(&(cb.len() as u32).to_be_bytes()[1..]);
                    msg.extend_from_slice(&cb);
                    h.send_enc_hs(&msg)?;
                    h.th.extend_from_slice(&msg);
                }

                let vd = h.hash.hmac(&h.fin_c, &h.hash.hash(&h.th));
                let mut fin = vec![20u8];
                fin.extend_from_slice(&(vd.len() as u32).to_be_bytes()[1..]);
                fin.extend_from_slice(&vd);
                h.send_enc_hs(&fin)?;
                h.th.extend_from_slice(&fin);
                h.tcp.write_all(&[20, 3, 3, 0, 1, 1]).ok(); // compat CCS

                let mut c_iv = [0u8; 12];
                let mut s_iv = [0u8; 12];
                let c_key = h.hash.expand_label(&c_ap, "key", &[], h.suite.key_len());
                c_iv.copy_from_slice(&h.hash.expand_label(&c_ap, "iv", &[], 12));
                let s_key = h.hash.expand_label(&s_ap, "key", &[], h.suite.key_len());
                s_iv.copy_from_slice(&h.hash.expand_label(&s_ap, "iv", &[], 12));
                return Ok(TlsStream {
                    tcp: h.tcp,
                    suite: h.suite,
                    hash: h.hash,
                    c_secret: c_ap,
                    s_secret: s_ap,
                    c_key,
                    c_iv,
                    s_key,
                    s_iv,
                    c_seq: 0,
                    s_seq: 0,
                    plain: Cursor::new(Vec::new()),
                    closed: false,
                });
            }
            4 => {}
            t2 => return Err(format!("unexpected hs msg {}", t2)),
        }
        h.th.extend_from_slice(&raw);
    }
}

impl H {
    /// Read next handshake message from the server during the encrypted
    /// flight. Returns (msg_type, body) and appends to `hs` queue as needed.
    fn read_hs_msg(&mut self) -> Result<(u8, Vec<u8>), String> {
        loop {
            if self.shs.hs.len() >= 4 {
                let len = u32::from_be_bytes([0, self.shs.hs[1], self.shs.hs[2], self.shs.hs[3]])
                    as usize;
                if self.shs.hs.len() >= 4 + len {
                    let msg: Vec<u8> = self.shs.hs.drain(..4 + len).collect();
                    return Ok((msg[0], msg[4..].to_vec()));
                }
            }
            // need more data — read next protected record
            let (ty, raw) = read_raw_record(&mut self.tcp)?;
            match ty {
                REC_CCS => continue,
                REC_ALERT => return Err(alert_str(&raw)),
                REC_APP => {
                    let mut aad = Vec::new();
                    aad.push(ty);
                    aad.extend_from_slice(&[0x03, 0x03]);
                    aad.extend_from_slice(&(raw.len() as u16).to_be_bytes());
                    let pt = self.suite.open(
                        &self.s_hs_key,
                        &make_nonce(&self.s_hs_iv, self.shs.seq),
                        &raw,
                        &aad,
                    )?;
                    self.shs.seq += 1;
                    if pt.is_empty() {
                        continue;
                    }
                    let inner = pt[pt.len() - 1];
                    let content = &pt[..pt.len() - 1];
                    match inner {
                        REC_HS => self.shs.hs.extend_from_slice(content),
                        REC_CCS => {}
                        REC_ALERT => return Err(alert_str(content)),
                        _ => {}
                    }
                }
                t2 => return Err(format!("unexpected record {}", t2)),
            }
        }
    }

    fn send_enc_hs(&mut self, msg: &[u8]) -> Result<(), String> {
        let mut inner = msg.to_vec();
        inner.push(REC_HS);
        let aad = hdr(REC_APP, inner.len() + 16);
        let sealed = self.suite.seal(
            &self.c_hs_key,
            &make_nonce(&self.c_hs_iv, self.c_hs_seq),
            &aad,
            &inner,
        );
        self.c_hs_seq += 1;
        let mut rec = aad;
        rec.extend_from_slice(&sealed);
        self.tcp.write_all(&rec).map_err(|e| e.to_string())
    }

    /// Read a ServerHello record; returns (raw handshake message, is_hrr).
    /// Side effects: sets `suite`, `hash`; extracts key share / HRR cookie.
    fn read_sh(&mut self) -> Result<(Vec<u8>, bool), String> {
        loop {
            let (ty, raw) = read_raw_record(&mut self.tcp)?;
            match ty {
                REC_CCS => continue,
                REC_ALERT => return Err(alert_str(&raw)),
                REC_HS => {
                    if raw.len() < 4 || raw[0] != 2 {
                        return Err("expected ServerHello".into());
                    }
                    let mlen = u32::from_be_bytes([0, raw[1], raw[2], raw[3]]) as usize + 4;
                    let msg = raw[..mlen.min(raw.len())].to_vec();
                    let mut r = Reader::new(&msg[4..]);
                    let _legacy_version = r.u16()?;
                    let random = r.take(32)?;
                    let is_hrr = random == HRR_RANDOM;
                    let _sid = r.vec8()?;
                    let suite_id = r.u16()?;
                    self.suite = Suite::from_id(suite_id).ok_or("unsupported cipher suite")?;
                    self.hash = self.suite.hash();
                    let _comp = r.u8()?;
                    let exts = r.vec16()?;
                    let mut er = Reader::new(exts);
                    while !er.done() {
                        let ety = er.u16()?;
                        let ebody = er.vec16()?;
                        match ety {
                            51 => {
                                // key_share
                                if is_hrr {
                                    let mut kr = Reader::new(ebody);
                                    let _group = kr.u16()?;
                                } else {
                                    let mut kr = Reader::new(ebody);
                                    let group = kr.u16()?;
                                    if group != 0x001d {
                                        return Err("unsupported group".into());
                                    }
                                    let share = kr.vec16()?;
                                    self.shared = Some(share.to_vec());
                                }
                            }
                            44 => self.cookie = Some(ebody.to_vec()),
                            43 => {} // supported_versions
                            _ => {}
                        }
                    }
                    return Ok((msg, is_hrr));
                }
                t2 => return Err(format!("expected hs record, got {}", t2)),
            }
        }
    }

    fn take_shared(&mut self) -> Result<Vec<u8>, String> {
        self.shared
            .take()
            .ok_or_else(|| "no server key share".into())
    }
    fn take_cookie(&mut self) -> Option<Vec<u8>> {
        self.cookie.take()
    }
}

/// Verify a TLS 1.3 CertificateVerify signature given the peer's leaf cert.
fn verify_cv(scheme: u16, msg: &[u8], sig: &[u8], cert: &x509::Cert) -> Result<bool, String> {
    use crate::bignum::Big;
    use crate::der;
    match (scheme, &cert.spki) {
        (
            0x0403,
            x509::Spki::Ec {
                curve: x509::EcCurve::P256,
                x,
                y,
            },
        )
        | (
            0x0503,
            x509::Spki::Ec {
                curve: x509::EcCurve::P384,
                x,
                y,
            },
        ) => {
            let (seq, _) = der::read_tlv(sig)?;
            if seq.tag != 0x30 {
                return Ok(false);
            }
            let mut it = der::children(seq.value);
            let r = it.next().ok_or("cv: no r")?;
            let s = it.next().ok_or("cv: no s")?;
            let (params, digest) = if scheme == 0x0403 {
                (curve::p256(), crate::sha2::sha256(msg).to_vec())
            } else {
                (curve::p384(), crate::sha2::sha384(msg).to_vec())
            };
            Ok(curve::ecdsa_verify(
                &params,
                x,
                y,
                &digest,
                &Big::from_be_bytes(der::int_bytes(r.value)),
                &Big::from_be_bytes(der::int_bytes(s.value)),
            ))
        }
        (0x0804, x509::Spki::Rsa { n, e }) => Ok(pss_msg_verify(
            n,
            e,
            sig,
            &crate::sha2::HashAlg::Sha256,
            &crate::sha2::sha256(msg),
        )),
        (0x0805, x509::Spki::Rsa { n, e }) => Ok(pss_msg_verify(
            n,
            e,
            sig,
            &crate::sha2::HashAlg::Sha384,
            &crate::sha2::sha384(msg),
        )),
        (0x0806, x509::Spki::Rsa { n, e }) => Ok(pss_msg_verify(
            n,
            e,
            sig,
            &crate::sha2::HashAlg::Sha512,
            &crate::sha2::sha512(msg),
        )),
        (0x0401 | 0x0501 | 0x0601, x509::Spki::Rsa { n, e }) => {
            let oid = match scheme {
                0x0401 => "1.2.840.113549.1.1.11",
                0x0501 => "1.2.840.113549.1.1.12",
                _ => "1.2.840.113549.1.1.13",
            };
            // reuse the cert-path verifier with a synthesized sig_alg
            x509::verify_signature(
                oid,
                msg,
                sig,
                &x509::Spki::Rsa {
                    n: n.clone(),
                    e: e.clone(),
                },
            )
        }
        _ => Err(format!("unsupported cv scheme {:#x}", scheme)),
    }
}

fn pss_msg_verify(
    n: &crate::bignum::Big,
    e: &crate::bignum::Big,
    sig: &[u8],
    alg: &HashAlg,
    mhash: &[u8],
) -> bool {
    // EMSA-PSS verify, salt length = hash length
    let k = n.bits().div_ceil(8);
    let s = crate::bignum::Big::from_be_bytes(sig);
    let em = s.modpow(e, n).to_be_bytes(k);
    if em.len() != k || em[k - 1] != 0xbc {
        return false;
    }
    let hlen = alg.out_len();
    if k < hlen + hlen + 2 {
        return false;
    }
    let db_len = k - hlen - 1;
    let (masked_db, h) = em.split_at(db_len);
    let mask = {
        let mut out = Vec::new();
        let mut ctr = 0u32;
        while out.len() < db_len {
            let mut m = h.to_vec();
            m.extend_from_slice(&ctr.to_be_bytes());
            out.extend_from_slice(&alg.hash(&m));
            ctr += 1;
        }
        out.truncate(db_len);
        out
    };
    let mut db = masked_db.to_vec();
    for i in 0..db_len {
        db[i] ^= mask[i];
    }
    let em_bits = n.bits() - 1;
    let unused = 8 * k - em_bits;
    if unused > 0 {
        db[0] &= 0xff >> unused;
    }
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
    alg.hash(&m) == h
}

fn build_client_hello(host: &str, priv_key: &[u8; 32], cookie: Option<&[u8]>) -> Vec<u8> {
    let mut ch = Vec::with_capacity(512);
    ch.extend_from_slice(&[0x03, 0x03]);
    ch.extend_from_slice(&rng::rand_bytes(32));
    let sid = rng::rand_bytes(32);
    ch.push(sid.len() as u8);
    ch.extend_from_slice(&sid);
    ch.extend_from_slice(&[0, 6, 0x13, 0x01, 0x13, 0x03, 0x13, 0x02]);
    ch.extend_from_slice(&[1, 0]);
    let mut ext = Vec::new();
    macro_rules! add {
        ($ty:expr, $body:expr) => {{
            let b: &[u8] = &$body;
            ext.extend_from_slice(&($ty as u16).to_be_bytes());
            ext.extend_from_slice(&(b.len() as u16).to_be_bytes());
            ext.extend_from_slice(b);
        }};
    }
    let hn = host.as_bytes();
    let mut sni = Vec::new();
    sni.extend_from_slice(&((hn.len() + 3) as u16).to_be_bytes());
    sni.push(0);
    sni.extend_from_slice(&(hn.len() as u16).to_be_bytes());
    sni.extend_from_slice(hn);
    add!(0, sni);
    add!(43, vec![2, 0x03, 0x04]);
    add!(10, vec![0, 4, 0x00, 0x1d, 0x00, 0x17]);
    let sigs: Vec<u8> = vec![
        0x04, 0x03, 0x05, 0x03, 0x08, 0x04, 0x08, 0x05, 0x08, 0x06, 0x04, 0x01, 0x05, 0x01, 0x06,
        0x01,
    ];
    let mut sb = Vec::new();
    sb.extend_from_slice(&(sigs.len() as u16).to_be_bytes());
    sb.extend_from_slice(&sigs);
    add!(13, sb);
    let pub_key = curve::x25519(priv_key, &[9u8; 32]);
    let mut ks = Vec::new();
    let mut ent = Vec::new();
    ent.extend_from_slice(&0x001du16.to_be_bytes());
    ent.extend_from_slice(&(pub_key.len() as u16).to_be_bytes());
    ent.extend_from_slice(&pub_key);
    ks.extend_from_slice(&(ent.len() as u16).to_be_bytes());
    ks.extend_from_slice(&ent);
    add!(51, ks);
    add!(45, vec![1, 1]);
    let mut alpn = Vec::new();
    alpn.extend_from_slice(&9u16.to_be_bytes());
    alpn.push(8);
    alpn.extend_from_slice(b"http/1.1");
    add!(16, alpn);
    if let Some(c) = cookie {
        let mut cb = Vec::new();
        cb.extend_from_slice(&(c.len() as u16).to_be_bytes());
        cb.extend_from_slice(c);
        add!(44, cb);
    }
    ch.extend_from_slice(&(ext.len() as u16).to_be_bytes());
    ch.extend_from_slice(&ext);
    let mut msg = vec![1u8];
    msg.extend_from_slice(&(ch.len() as u32).to_be_bytes()[1..]);
    msg.extend_from_slice(&ch);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Duration;

    struct Fragmented<'a> {
        bytes: &'a [u8],
        interrupt: bool,
    }

    impl Read for Fragmented<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let len = buf.len().min(1);
            self.bytes.read(&mut buf[..len])
        }
    }

    #[test]
    fn records_handle_short_and_interrupted_reads() {
        let bytes = record(REC_APP, b"hello");
        let mut reader = Fragmented {
            bytes: &bytes,
            interrupt: false,
        };
        let (ty, body) = read_raw_record(&mut reader).unwrap();
        assert_eq!(ty, REC_APP);
        assert_eq!(body, b"hello");
    }

    #[test]
    fn records_reject_truncation() {
        let bytes = record(REC_APP, b"hello");
        for len in 0..bytes.len() {
            assert!(
                read_raw_record(&mut &bytes[..len]).is_err(),
                "accepted {len} bytes"
            );
        }
    }

    #[test]
    fn records_enforce_size_limit_and_preserve_next_record() {
        let mut bytes = record(REC_APP, &vec![0; 18432]);
        bytes.extend(record(REC_ALERT, b"\x01\x00"));
        let mut reader = bytes.as_slice();
        assert_eq!(read_raw_record(&mut reader).unwrap().1.len(), 18432);
        assert_eq!(
            read_raw_record(&mut reader).unwrap(),
            (REC_ALERT, vec![1, 0])
        );
        assert!(reader.is_empty());

        let oversized = hdr(REC_APP, 18433);
        assert_eq!(
            read_raw_record(&mut oversized.as_slice()).unwrap_err(),
            "record too large"
        );
    }

    fn stream_pair() -> (TlsStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        tcp.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        let (peer, _) = listener.accept().unwrap();
        let suite = Suite::Aes128GcmSha256;
        (
            TlsStream {
                tcp,
                suite,
                hash: suite.hash(),
                c_secret: vec![],
                s_secret: vec![],
                c_key: vec![0; 16],
                c_iv: [0; 12],
                s_key: vec![0; 16],
                s_iv: [0; 12],
                c_seq: 0,
                s_seq: 0,
                plain: Cursor::new(Vec::new()),
                closed: false,
            },
            peer,
        )
    }

    #[test]
    fn empty_buffers_do_not_touch_the_transport() {
        let (mut stream, _peer) = stream_pair();
        stream.tcp.set_nonblocking(true).unwrap();
        assert_eq!(stream.read(&mut []).unwrap(), 0);
        assert_eq!(stream.write(&[]).unwrap(), 0);
        assert_eq!(stream.c_seq, 0);
        assert_eq!(stream.s_seq, 0);
    }

    #[test]
    fn partial_reads_preserve_buffered_plaintext() {
        let (mut stream, _peer) = stream_pair();
        stream.plain = Cursor::new(b"hello".to_vec());
        stream.closed = true;
        let mut buf = [0; 2];
        assert_eq!(stream.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf, b"he");
        assert_eq!(stream.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf, b"ll");
        assert_eq!(stream.read(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], b'o');
        assert_eq!(stream.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn empty_application_records_do_not_signal_eof() {
        let (mut stream, mut peer) = stream_pair();
        for (seq, content) in [b"".as_slice(), b"he", b"llo"].iter().enumerate() {
            let mut inner = content.to_vec();
            inner.push(REC_APP);
            let aad = hdr(REC_APP, inner.len() + 16);
            let ciphertext = stream.suite.seal(
                &stream.s_key,
                &make_nonce(&stream.s_iv, seq as u64),
                &aad,
                &inner,
            );
            peer.write_all(&aad).unwrap();
            peer.write_all(&ciphertext).unwrap();
        }
        let mut buf = [0; 5];
        stream.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        assert_eq!(stream.s_seq, 3);
    }
}
