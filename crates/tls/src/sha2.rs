//! SHA-256 / SHA-384, HMAC, HKDF, and the TLS 1.3 key schedule
//! (Derive-Secret over the running transcript hash).

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    for block in pad_blocks(data, 64, 8) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K256[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

const K512: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

fn sha512_core(data: &[u8], h0: &[u64; 8], out_len: usize) -> Vec<u8> {
    let mut h = *h0;
    for block in pad_blocks(data, 128, 16) {
        let mut w = [0u64; 80];
        for i in 0..16 {
            w[i] = u64::from_be_bytes([
                block[i * 8],
                block[i * 8 + 1],
                block[i * 8 + 2],
                block[i * 8 + 3],
                block[i * 8 + 4],
                block[i * 8 + 5],
                block[i * 8 + 6],
                block[i * 8 + 7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K512[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (i, v) in [a, b, c, d, e, f, g, hh].iter().enumerate() {
            h[i] = h[i].wrapping_add(*v);
        }
    }
    let mut out = Vec::with_capacity(64);
    for v in h {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.truncate(out_len);
    out
}

pub fn sha384(data: &[u8]) -> [u8; 48] {
    const H: [u64; 8] = [
        0xcbbb9d5dc1059ed8,
        0x629a292a367cd507,
        0x9159015a3070dd17,
        0x152fecd8f70e5939,
        0x67332667ffc00b31,
        0x8eb44a8768581511,
        0xdb0c2e0d64f98fa7,
        0x47b5481dbefa4fa4,
    ];
    let v = sha512_core(data, &H, 48);
    let mut o = [0u8; 48];
    o.copy_from_slice(&v);
    o
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    const H: [u64; 8] = [
        0x6a09e667f3bcc908,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];
    let v = sha512_core(data, &H, 64);
    let mut o = [0u8; 64];
    o.copy_from_slice(&v);
    o
}

/// Generic padding splitter for Merkle-Damgård: returns Vec of blocks.
fn pad_blocks(data: &[u8], block: usize, len_bytes: usize) -> Vec<Vec<u8>> {
    let bit_len = (data.len() as u128) * 8;
    let mut v = data.to_vec();
    v.push(0x80);
    while !(v.len() + len_bytes).is_multiple_of(block) {
        v.push(0);
    }
    v.extend_from_slice(&bit_len.to_be_bytes()[16 - len_bytes..]);
    v.chunks(block).map(|c| c.to_vec()).collect()
}

// --------------- HMAC / HKDF ---------------

pub enum HashAlg {
    Sha256,
    Sha384,
    Sha512,
}

impl HashAlg {
    pub fn out_len(&self) -> usize {
        match self {
            HashAlg::Sha256 => 32,
            HashAlg::Sha384 => 48,
            HashAlg::Sha512 => 64,
        }
    }
    pub fn block_len(&self) -> usize {
        match self {
            HashAlg::Sha256 => 64,
            HashAlg::Sha384 | HashAlg::Sha512 => 128,
        }
    }
    pub fn hash(&self, data: &[u8]) -> Vec<u8> {
        match self {
            HashAlg::Sha256 => sha256(data).to_vec(),
            HashAlg::Sha384 => sha384(data).to_vec(),
            HashAlg::Sha512 => sha512(data).to_vec(),
        }
    }
    pub fn hmac(&self, key: &[u8], msg: &[u8]) -> Vec<u8> {
        let bl = self.block_len();
        let mut k = if key.len() > bl {
            self.hash(key)
        } else {
            key.to_vec()
        };
        k.resize(bl, 0);
        let mut inner = Vec::with_capacity(bl + msg.len());
        for &b in &k {
            inner.push(b ^ 0x36);
        }
        inner.extend_from_slice(msg);
        let ih = self.hash(&inner);
        let mut outer = Vec::with_capacity(bl + ih.len());
        for &b in &k {
            outer.push(b ^ 0x5c);
        }
        outer.extend_from_slice(&ih);
        self.hash(&outer)
    }
    pub fn hkdf_extract(&self, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
        let s = if salt.is_empty() {
            vec![0u8; self.out_len()]
        } else {
            salt.to_vec()
        };
        self.hmac(&s, ikm)
    }
    pub fn hkdf_expand(&self, prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut t: Vec<u8> = Vec::new();
        let mut i = 1u8;
        while out.len() < len {
            let mut m = t.clone();
            m.extend_from_slice(info);
            m.push(i);
            t = self.hmac(prk, &m);
            out.extend_from_slice(&t);
            i += 1;
        }
        out.truncate(len);
        out
    }
    /// TLS 1.3 HKDF-Expand-Label.
    pub fn expand_label(&self, secret: &[u8], label: &str, ctx: &[u8], len: usize) -> Vec<u8> {
        let mut info = Vec::new();
        info.extend_from_slice(&(len as u16).to_be_bytes());
        let l = format!("tls13 {}", label);
        info.push(l.len() as u8);
        info.extend_from_slice(l.as_bytes());
        info.push(ctx.len() as u8);
        info.extend_from_slice(ctx);
        self.hkdf_expand(secret, &info, len)
    }
    /// TLS 1.3 Derive-Secret(Secret, Label, Messages) — caller passes the
    /// transcript hash of Messages.
    pub fn derive_secret(&self, secret: &[u8], label: &str, transcript_hash: &[u8]) -> Vec<u8> {
        self.expand_label(secret, label, transcript_hash, self.out_len())
    }
}

/// Running handshake transcript.
pub struct Transcript {
    pub alg: HashAlg,
    pub data: Vec<u8>,
}

impl Transcript {
    pub fn new(alg: HashAlg) -> Self {
        Self {
            alg,
            data: Vec::new(),
        }
    }
    pub fn add(&mut self, msg: &[u8]) {
        self.data.extend_from_slice(msg);
    }
    pub fn hash(&self) -> Vec<u8> {
        self.alg.hash(&self.data)
    }
    /// Rebuild transcript for HelloRetryRequest: replace CH with
    /// message_hash(ClientHello1) per RFC 8446 §4.4.1.
    pub fn apply_hrr(&mut self) {
        let h = self.hash();
        let mut d = Vec::new();
        d.push(254); // message_hash
        d.extend_from_slice(&(h.len() as u32).to_be_bytes()[1..]);
        d.extend_from_slice(&h);
        self.data = d;
    }
    pub fn clone_hash_for(&self, extra: &[u8]) -> Vec<u8> {
        let mut d = self.data.clone();
        d.extend_from_slice(extra);
        self.alg.hash(&d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vec() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha384_vec() {
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
    }

    #[test]
    fn hmac_vec() {
        // RFC 4231 test case 1
        let a = HashAlg::Sha256;
        let key = [0x0b; 20];
        let m = a.hmac(&key, b"Hi There");
        assert_eq!(
            hex(&m),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn hkdf_vec() {
        // RFC 5869 test case 1 (SHA-256)
        let a = HashAlg::Sha256;
        let ikm = [0x0b; 22];
        let salt: Vec<u8> = (0..13).collect();
        let info: Vec<u8> = (0xf0..0xfa).collect();
        let prk = a.hkdf_extract(&salt, &ikm);
        assert_eq!(
            hex(&prk),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let okm = a.hkdf_expand(&prk, &info, 42);
        assert_eq!(
            hex(&okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }
}
