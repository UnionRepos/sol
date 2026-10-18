//! AEAD ciphers for TLS 1.3: AES-128/256-GCM and ChaCha20-Poly1305.

// ---------------- AES ----------------

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const RCON: [u32; 15] = [
    0x00000000, 0x01000000, 0x02000000, 0x04000000, 0x08000000, 0x10000000, 0x20000000, 0x40000000,
    0x80000000, 0x1b000000, 0x36000000, 0x6c000000, 0xd8000000, 0xab000000, 0x4d000000,
];

pub struct Aes {
    rk: Vec<u32>,
    rounds: usize,
}

impl Aes {
    pub fn new(key: &[u8]) -> Aes {
        let nk = key.len() / 4;
        let rounds = nk + 6;
        let mut rk = Vec::with_capacity(4 * (rounds + 1));
        for i in 0..nk {
            rk.push(u32::from_be_bytes([
                key[4 * i],
                key[4 * i + 1],
                key[4 * i + 2],
                key[4 * i + 3],
            ]));
        }
        for i in nk..4 * (rounds + 1) {
            let mut t = rk[i - 1];
            if i % nk == 0 {
                t = sub_word(t.rotate_left(8)) ^ RCON[i / nk];
            } else if nk > 6 && i % nk == 4 {
                t = sub_word(t);
            }
            rk.push(rk[i - nk] ^ t);
        }
        Aes { rk, rounds }
    }

    pub fn encrypt_block(&self, inp: &[u8; 16]) -> [u8; 16] {
        let mut s = [0u8; 16];
        for i in 0..16 {
            s[i] = inp[i] ^ (self.rk[i / 4] >> ((3 - i % 4) * 8)) as u8;
        }
        for r in 1..self.rounds {
            // sub bytes + shift rows (state is column-major per FIPS-197)
            let mut t = [0u8; 16];
            for c in 0..4 {
                for rr in 0..4 {
                    t[c * 4 + rr] = SBOX[s[((c + rr) % 4) * 4 + rr] as usize];
                }
            }
            // mix columns
            for c in 0..4 {
                let col = &mut t[c * 4..c * 4 + 4];
                let (a0, a1, a2, a3) = (col[0], col[1], col[2], col[3]);
                col[0] = xtime(a0) ^ (xtime(a1) ^ a1) ^ a2 ^ a3;
                col[1] = a0 ^ xtime(a1) ^ (xtime(a2) ^ a2) ^ a3;
                col[2] = a0 ^ a1 ^ xtime(a2) ^ (xtime(a3) ^ a3);
                col[3] = (xtime(a0) ^ a0) ^ a1 ^ a2 ^ xtime(a3);
            }
            for i in 0..16 {
                s[i] = t[i] ^ (self.rk[r * 4 + i / 4] >> ((3 - i % 4) * 8)) as u8;
            }
        }
        // final round: sub+shift only
        let mut t = [0u8; 16];
        for c in 0..4 {
            for rr in 0..4 {
                t[c * 4 + rr] = SBOX[s[((c + rr) % 4) * 4 + rr] as usize];
            }
        }
        for i in 0..16 {
            t[i] ^= (self.rk[self.rounds * 4 + i / 4] >> ((3 - i % 4) * 8)) as u8;
        }
        t
    }
}

fn sub_word(w: u32) -> u32 {
    let b = w.to_be_bytes();
    u32::from_be_bytes([
        SBOX[b[0] as usize],
        SBOX[b[1] as usize],
        SBOX[b[2] as usize],
        SBOX[b[3] as usize],
    ])
}

fn xtime(x: u8) -> u8 {
    if x & 0x80 != 0 {
        (x << 1) ^ 0x1b
    } else {
        x << 1
    }
}

// ---------------- GHASH ----------------

/// GF(2^128) multiply, polynomial x^128+x^7+x^2+x+1, bit-reflected.
fn gmul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if (x >> (127 - i)) & 1 == 1 {
            z ^= v;
        }
        if v & 1 == 1 {
            v = (v >> 1) ^ 0xe100_0000_0000_0000_0000_0000_0000_0000;
        } else {
            v >>= 1;
        }
    }
    z
}

fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> u128 {
    let mut y = 0u128;
    let feed = |y: &mut u128, d: &[u8]| {
        for chunk in d.chunks(16) {
            let mut b = [0u8; 16];
            b[..chunk.len()].copy_from_slice(chunk);
            *y = gmul(*y ^ u128::from_be_bytes(b), h);
        }
    };
    feed(&mut y, aad);
    feed(&mut y, ct);
    let mut lb = [0u8; 16];
    lb[..8].copy_from_slice(&((aad.len() as u64 * 8).to_be_bytes()));
    lb[8..].copy_from_slice(&((ct.len() as u64 * 8).to_be_bytes()));
    gmul(y ^ u128::from_be_bytes(lb), h)
}

pub struct AesGcm {
    aes: Aes,
    h: u128,
}

impl AesGcm {
    pub fn new(key: &[u8]) -> AesGcm {
        let aes = Aes::new(key);
        let h = u128::from_be_bytes(aes.encrypt_block(&[0u8; 16]));
        AesGcm { aes, h }
    }

    fn ctr_crypt(&self, icb: &[u8; 16], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len());
        let mut ctr = u128::from_be_bytes(*icb);
        for chunk in data.chunks(16) {
            ctr = ctr.wrapping_add(1);
            let ks = self.aes.encrypt_block(&ctr.to_be_bytes());
            for (i, &b) in chunk.iter().enumerate() {
                out.push(b ^ ks[i]);
            }
        }
        out
    }

    pub fn seal(&self, nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;
        let ct = self.ctr_crypt(&j0, pt);
        let s = ghash(self.h, aad, &ct);
        let tag_mask = self.aes.encrypt_block(&j0);
        let tag = s ^ u128::from_be_bytes(tag_mask);
        let mut out = ct;
        out.extend_from_slice(&tag.to_be_bytes());
        out
    }

    pub fn open(&self, nonce: &[u8; 12], aad: &[u8], ct_tag: &[u8]) -> Result<Vec<u8>, String> {
        if ct_tag.len() < 16 {
            return Err("gcm: too short".into());
        }
        let (ct, tag) = ct_tag.split_at(ct_tag.len() - 16);
        let mut j0 = [0u8; 16];
        j0[..12].copy_from_slice(nonce);
        j0[15] = 1;
        let s = ghash(self.h, aad, ct);
        let tag_mask = self.aes.encrypt_block(&j0);
        let expect = (s ^ u128::from_be_bytes(tag_mask)).to_be_bytes();
        if !ct_eq(&expect, tag) {
            return Err("gcm: tag mismatch".into());
        }
        Ok(self.ctr_crypt(&j0, ct))
    }
}

// ---------------- ChaCha20-Poly1305 ----------------

fn chacha_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut st = [0u32; 16];
    st[0] = 0x61707865;
    st[1] = 0x3320646e;
    st[2] = 0x79622d32;
    st[3] = 0x6b206574;
    for i in 0..8 {
        st[4 + i] =
            u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
    }
    st[12] = counter;
    for i in 0..3 {
        st[13 + i] = u32::from_le_bytes([
            nonce[i * 4],
            nonce[i * 4 + 1],
            nonce[i * 4 + 2],
            nonce[i * 4 + 3],
        ]);
    }
    let orig = st;
    for _ in 0..10 {
        // column rounds
        qr(&mut st, 0, 4, 8, 12);
        qr(&mut st, 1, 5, 9, 13);
        qr(&mut st, 2, 6, 10, 14);
        qr(&mut st, 3, 7, 11, 15);
        // diagonal rounds
        qr(&mut st, 0, 5, 10, 15);
        qr(&mut st, 1, 6, 11, 12);
        qr(&mut st, 2, 7, 8, 13);
        qr(&mut st, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        let v = st[i].wrapping_add(orig[i]);
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] ^= s[a];
    s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] ^= s[c];
    s[b] = s[b].rotate_left(7);
}

fn chacha20_crypt(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut ctr = counter;
    for chunk in data.chunks(64) {
        let ks = chacha_block(key, ctr, nonce);
        ctr = ctr.wrapping_add(1);
        for (i, &b) in chunk.iter().enumerate() {
            out.push(b ^ ks[i]);
        }
    }
    out
}

/// Poly1305 one-time authenticator (RFC 8439 §2.5). 44/44/42-bit limbs,
/// same layout as poly1305-donna-64.
fn poly1305(msg: &[u8], key: &[u8; 32]) -> [u8; 16] {
    let mut r = [0u8; 16];
    r.copy_from_slice(&key[..16]);
    r[3] &= 15;
    r[7] &= 15;
    r[11] &= 15;
    r[15] &= 15;
    r[4] &= 252;
    r[8] &= 252;
    r[12] &= 252;
    let r_big = u128::from_le_bytes(r);
    let r0 = (r_big & 0xfffffffffff) as u64;
    let r1 = ((r_big >> 44) & 0xfffffffffff) as u64;
    let r2 = ((r_big >> 88) & 0x3ffffffffff) as u64;
    let (s1, s2) = (r1 * 20, r2 * 20);
    let mut h0 = 0u64;
    let mut h1 = 0u64;
    let mut h2 = 0u64;
    let mask44 = 0xfffffffffffu64;
    let mask42 = 0x3ffffffffffu64;
    for chunk in msg.chunks(16) {
        let mut b = [0u8; 17];
        b[..chunk.len()].copy_from_slice(chunk);
        b[chunk.len()] = 1;
        // low 16 bytes → u128; the 17th byte lands at bit 128 (limb2 bit 40)
        let mut n = 0u128;
        for (i, &x) in b[..16].iter().enumerate() {
            n |= (x as u128) << (i * 8);
        }
        h0 += (n & 0xfffffffffff) as u64;
        h1 += ((n >> 44) & 0xfffffffffff) as u64;
        h2 += ((n >> 88) & 0x3ffffffffff) as u64 + ((b[16] as u64) << 40);
        // h = (h + m) * r mod (2^130 - 5); limb idx 3 folds *20, idx 4 *20<<44
        let d0 = h0 as u128 * r0 as u128 + h1 as u128 * s2 as u128 + h2 as u128 * s1 as u128;
        let d1 = h0 as u128 * r1 as u128 + h1 as u128 * r0 as u128 + h2 as u128 * s2 as u128;
        let d2 = h0 as u128 * r2 as u128 + h1 as u128 * r1 as u128 + h2 as u128 * r0 as u128;
        let mut c = d0 >> 44;
        h0 = (d0 as u64) & mask44;
        let d1 = d1 + c;
        c = d1 >> 44;
        h1 = (d1 as u64) & mask44;
        let d2 = d2 + c;
        c = d2 >> 42;
        h2 = (d2 as u64) & mask42;
        h0 += (c as u64) * 5;
        let c2 = h0 >> 44;
        h0 &= mask44;
        h1 += c2;
    }
    // final carry propagate
    let mut c = h1 >> 44;
    h1 &= mask44;
    h2 += c;
    c = h2 >> 42;
    h2 &= mask42;
    h0 += c * 5;
    c = h0 >> 44;
    h0 &= mask44;
    h1 += c;
    c = h1 >> 44;
    h1 &= mask44;
    h2 += c;
    // g = h + 5 - 2^130; select g if h >= p
    let g0 = h0 + 5;
    let c0 = g0 >> 44;
    let g0 = g0 & mask44;
    let g1 = h1 + c0;
    let c1 = g1 >> 44;
    let g1 = g1 & mask44;
    let g2 = h2.wrapping_add(c1).wrapping_sub(1 << 42);
    let sel = (g2 >> 63).wrapping_sub(1); // all-ones iff no borrow (h >= p)
    let h0 = (h0 & !sel) | (g0 & sel);
    let h1 = (h1 & !sel) | (g1 & sel);
    let h2 = (h2 & !sel) | (g2 & sel & mask42);
    // serialize h mod 2^128, add s
    let hval = (h0 as u128) | ((h1 as u128) << 44) | ((h2 as u128) << 88);
    let s = u128::from_le_bytes(key[16..32].try_into().unwrap());
    hval.wrapping_add(s).to_le_bytes()
}

pub struct ChaCha20Poly1305 {
    key: [u8; 32],
}

impl ChaCha20Poly1305 {
    pub fn new(key: &[u8]) -> ChaCha20Poly1305 {
        let mut k = [0u8; 32];
        k.copy_from_slice(&key[..32]);
        ChaCha20Poly1305 { key: k }
    }

    fn poly_key(&self, nonce: &[u8; 12]) -> [u8; 32] {
        let b = chacha_block(&self.key, 0, nonce);
        let mut k = [0u8; 32];
        k.copy_from_slice(&b[..32]);
        k
    }

    fn aead_mac(&self, aad: &[u8], ct: &[u8], key: &[u8; 32]) -> [u8; 16] {
        let mut m = Vec::with_capacity(aad.len() + ct.len() + 32);
        m.extend_from_slice(aad);
        m.resize(m.len() + (16 - aad.len() % 16) % 16, 0);
        m.extend_from_slice(ct);
        m.resize(m.len() + (16 - ct.len() % 16) % 16, 0);
        m.extend_from_slice(&(aad.len() as u64).to_le_bytes());
        m.extend_from_slice(&(ct.len() as u64).to_le_bytes());
        poly1305(&m, key)
    }

    pub fn seal(&self, nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
        let ct = chacha20_crypt(&self.key, 1, nonce, pt);
        let pk = self.poly_key(nonce);
        let tag = self.aead_mac(aad, &ct, &pk);
        let mut out = ct;
        out.extend_from_slice(&tag);
        out
    }

    pub fn open(&self, nonce: &[u8; 12], aad: &[u8], ct_tag: &[u8]) -> Result<Vec<u8>, String> {
        if ct_tag.len() < 16 {
            return Err("poly1305: too short".into());
        }
        let (ct, tag) = ct_tag.split_at(ct_tag.len() - 16);
        let pk = self.poly_key(nonce);
        let expect = self.aead_mac(aad, ct, &pk);
        if !ct_eq(&expect, tag) {
            return Err("poly1305: tag mismatch".into());
        }
        Ok(chacha20_crypt(&self.key, 1, nonce, ct))
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut x = 0u8;
    for i in 0..a.len() {
        x |= a[i] ^ b[i];
    }
    x == 0
}

// ---------------- TLS 1.3 suite abstraction ----------------

#[derive(Clone, Copy, PartialEq)]
pub enum Suite {
    Aes128GcmSha256,
    Aes256GcmSha384,
    Chacha20Poly1305Sha256,
}

impl Suite {
    pub fn from_id(id: u16) -> Option<Suite> {
        match id {
            0x1301 => Some(Suite::Aes128GcmSha256),
            0x1302 => Some(Suite::Aes256GcmSha384),
            0x1303 => Some(Suite::Chacha20Poly1305Sha256),
            _ => None,
        }
    }
    pub fn key_len(&self) -> usize {
        match self {
            Suite::Aes128GcmSha256 => 16,
            Suite::Aes256GcmSha384 => 32,
            Suite::Chacha20Poly1305Sha256 => 32,
        }
    }
    pub fn hash(&self) -> crate::sha2::HashAlg {
        match self {
            Suite::Aes256GcmSha384 => crate::sha2::HashAlg::Sha384,
            _ => crate::sha2::HashAlg::Sha256,
        }
    }
    pub fn seal(&self, key: &[u8], nonce: &[u8; 12], aad: &[u8], pt: &[u8]) -> Vec<u8> {
        match self {
            Suite::Chacha20Poly1305Sha256 => ChaCha20Poly1305::new(key).seal(nonce, aad, pt),
            _ => AesGcm::new(key).seal(nonce, aad, pt),
        }
    }
    pub fn open(
        &self,
        key: &[u8],
        nonce: &[u8; 12],
        aad: &[u8],
        ct: &[u8],
    ) -> Result<Vec<u8>, String> {
        match self {
            Suite::Chacha20Poly1305Sha256 => ChaCha20Poly1305::new(key).open(nonce, aad, ct),
            _ => AesGcm::new(key).open(nonce, aad, ct),
        }
    }
}

pub fn make_nonce(iv: &[u8; 12], seq: u64) -> [u8; 12] {
    let mut n = *iv;
    let s = seq.to_be_bytes();
    for i in 0..8 {
        n[4 + i] ^= s[i];
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_vector() {
        // FIPS-197 appendix B: key 2b7e151628aed2a6abf7158809cf4f3c
        let key: Vec<u8> = (0..16)
            .map(|i| {
                [
                    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09,
                    0xcf, 0x4f, 0x3c,
                ][i]
            })
            .collect();
        let aes = Aes::new(&key);
        let pt: [u8; 16] = [
            0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d, 0x31, 0x31, 0x98, 0xa2, 0xe0, 0x37,
            0x07, 0x34,
        ];
        let ct = aes.encrypt_block(&pt);
        let expect: [u8; 16] = [
            0x39, 0x25, 0x84, 0x1d, 0x02, 0xdc, 0x09, 0xfb, 0xdc, 0x11, 0x85, 0x97, 0x19, 0x6a,
            0x0b, 0x32,
        ];
        assert_eq!(ct, expect);
    }

    #[test]
    fn gcm_vector() {
        // NIST GCM test: zero key, zero iv, zero pt => known tag
        let g = AesGcm::new(&[0u8; 16]);
        let out = g.seal(&[0u8; 12], &[], &[0u8; 16]);
        // CT = 0388dace60b6a392f328c2b971b2fe78, tag = ab6e47d42cec13bdf53a67b21257bddf
        let expect: Vec<u8> = vec![
            0x03, 0x88, 0xda, 0xce, 0x60, 0xb6, 0xa3, 0x92, 0xf3, 0x28, 0xc2, 0xb9, 0x71, 0xb2,
            0xfe, 0x78, 0xab, 0x6e, 0x47, 0xd4, 0x2c, 0xec, 0x13, 0xbd, 0xf5, 0x3a, 0x67, 0xb2,
            0x12, 0x57, 0xbd, 0xdf,
        ];
        assert_eq!(out, expect);
        assert_eq!(g.open(&[0u8; 12], &[], &expect).unwrap(), vec![0u8; 16]);
    }

    #[test]
    fn chacha_block_vector() {
        // RFC 8439 §2.3.2 test vector
        let key: [u8; 32] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
            0x1c, 0x1d, 0x1e, 0x1f,
        ];
        let nonce: [u8; 12] = [
            0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x00, 0x00, 0x00,
        ];
        let b = chacha_block(&key, 1, &nonce);
        let expect: [u8; 64] = [
            0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20,
            0x71, 0xc4, 0xc7, 0xd1, 0xf4, 0xc7, 0x33, 0xc0, 0x68, 0x03, 0x04, 0x22, 0xaa, 0x9a,
            0xc3, 0xd4, 0x6c, 0x4e, 0xd2, 0x82, 0x64, 0x46, 0x07, 0x9f, 0xaa, 0x09, 0x14, 0xc2,
            0xd7, 0x05, 0xd9, 0x8b, 0x02, 0xa2, 0xb5, 0x12, 0x9c, 0xd1, 0xde, 0x16, 0x4e, 0xb9,
            0xcb, 0xd0, 0x83, 0xe8, 0xa2, 0x50, 0x3c, 0x4e,
        ];
        assert_eq!(b, expect);
    }

    #[test]
    fn poly1305_vector() {
        // RFC 8439 §2.5.2
        let key: [u8; 32] = [
            0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5,
            0x06, 0xa8, 0x01, 0x03, 0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf,
            0x41, 0x49, 0xf5, 0x1b,
        ];
        let msg = b"Cryptographic Forum Research Group";
        let tag = poly1305(msg, &key);
        let expect: [u8; 16] = [
            0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01,
            0x27, 0xa9,
        ];
        assert_eq!(tag, expect);
    }

    #[test]
    fn chacha_aead_vector() {
        // RFC 8439 §2.8.2 — seal then open roundtrip with known tag
        let key: [u8; 32] = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];
        let nonce: [u8; 12] = [
            0x07, 0x00, 0x00, 0x00, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
        ];
        let aad: [u8; 12] = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let c = ChaCha20Poly1305::new(&key);
        let sealed = c.seal(&nonce, &aad, pt);
        let tag = &sealed[sealed.len() - 16..];
        let expect_tag: [u8; 16] = [
            0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60,
            0x06, 0x91,
        ];
        assert_eq!(tag, expect_tag);
        assert_eq!(c.open(&nonce, &aad, &sealed).unwrap(), pt);
    }
}
