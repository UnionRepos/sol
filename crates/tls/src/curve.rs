//! Curve25519 (X25519, RFC 7748) and ECDSA signature verification over
//! NIST P-256 / P-384 using Jacobian coordinates. Field arithmetic reuses
//! `bignum::Big` — a few hundred µs per operation is plenty for a
//! once-per-connection handshake.

use crate::bignum::Big;

fn p25519() -> Big {
    // 2^255 - 19
    let mut b = Big::one();
    for _ in 0..255 {
        b = b.shl1();
    }
    b.sub(&Big::from_u64(19))
}

fn fe_add(a: &Big, b: &Big, p: &Big) -> Big {
    a.add(b).rem(p)
}
fn fe_sub(a: &Big, b: &Big, p: &Big) -> Big {
    a.sub_mod(b, p)
}
fn fe_mul(a: &Big, b: &Big, p: &Big) -> Big {
    a.mul(b).rem(p)
}
fn fe_sq(a: &Big, p: &Big) -> Big {
    a.mul(a).rem(p)
}
fn fe_inv(a: &Big, p: &Big) -> Big {
    a.modpow(&p.sub(&Big::from_u64(2)), p)
}
fn fe_mul_small(a: &Big, k: u64, p: &Big) -> Big {
    a.mul(&Big::from_u64(k)).rem(p)
}

/// X25519 scalar multiplication (RFC 7748 §5).
pub fn x25519(scalar: &[u8; 32], u: &[u8; 32]) -> [u8; 32] {
    let p = p25519();
    let mut k = *scalar;
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let mut u_bytes = *u;
    u_bytes[31] &= 127; // clear high bit for X25519
    let x1 = Big::from_be_bytes(&{
        let mut r = u_bytes;
        r.reverse();
        r
    });
    let mut x2 = Big::one();
    let mut z2 = Big::zero();
    let mut x3 = x1.clone();
    let mut z3 = Big::one();
    let mut swap = false;
    for t in (0..255).rev() {
        let kt = k[t / 8] & (1 << (t % 8)) != 0;
        if swap != kt {
            std::mem::swap(&mut x2, &mut x3);
            std::mem::swap(&mut z2, &mut z3);
        }
        swap = kt;
        let a = fe_add(&x2, &z2, &p);
        let aa = fe_sq(&a, &p);
        let b = fe_sub(&x2, &z2, &p);
        let bb = fe_sq(&b, &p);
        let e = fe_sub(&aa, &bb, &p);
        let c = fe_add(&x3, &z3, &p);
        let d = fe_sub(&x3, &z3, &p);
        let da = fe_mul(&d, &a, &p);
        let cb = fe_mul(&c, &b, &p);
        let dacbp = fe_add(&da, &cb, &p);
        let dacbm = fe_sub(&da, &cb, &p);
        x3 = fe_sq(&dacbp, &p);
        z3 = fe_mul(&x1, &fe_sq(&dacbm, &p), &p);
        x2 = fe_mul(&aa, &bb, &p);
        z2 = fe_mul(&e, &fe_add(&aa, &fe_mul_small(&e, 121665, &p), &p), &p);
    }
    if swap {
        std::mem::swap(&mut x2, &mut x3);
        std::mem::swap(&mut z2, &mut z3);
    }
    let out = fe_mul(&x2, &fe_inv(&z2, &p), &p);
    let mut b = out.to_be_bytes(32);
    b.reverse();
    let mut r = [0u8; 32];
    r.copy_from_slice(&b);
    r
}

// ----------------- ECDSA -----------------

pub struct EcParams {
    pub p: Big,
    pub a: Big, // curve coefficient (p-3)
    pub b: Big,
    pub n: Big,
    pub gx: Big,
    pub gy: Big,
}

fn hexbig(s: &str) -> Big {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes: Vec<u8> = (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
        .collect();
    Big::from_be_bytes(&bytes)
}

pub fn p256() -> EcParams {
    let p = hexbig("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff");
    EcParams {
        a: p.sub(&Big::from_u64(3)),
        b: hexbig("5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b"),
        n: hexbig("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551"),
        gx: hexbig("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"),
        gy: hexbig("4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"),
        p,
    }
}

pub fn p384() -> EcParams {
    let p = hexbig(
        "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
    );
    EcParams {
        a: p.sub(&Big::from_u64(3)),
        b: hexbig(
            "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
        ),
        n: hexbig(
            "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
        ),
        gx: hexbig(
            "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
        ),
        gy: hexbig(
            "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
        ),
        p,
    }
}

/// Jacobian point. Infinity represented by z == 0.
#[derive(Clone)]
struct JPoint {
    x: Big,
    y: Big,
    z: Big,
}

impl JPoint {
    fn inf() -> JPoint {
        JPoint {
            x: Big::one(),
            y: Big::one(),
            z: Big::zero(),
        }
    }
    fn affine(x: Big, y: Big) -> JPoint {
        JPoint {
            x,
            y,
            z: Big::one(),
        }
    }
    fn is_inf(&self) -> bool {
        self.z.is_zero()
    }

    fn double(&self, c: &EcParams) -> JPoint {
        if self.is_inf() || self.y.is_zero() {
            return JPoint::inf();
        }
        let p = &c.p;
        let xx = fe_sq(&self.x, p);
        let yy = fe_sq(&self.y, p);
        let yyyy = fe_sq(&yy, p);
        let zz = fe_sq(&self.z, p);
        // S = 4*x*y^2
        let s = fe_mul_small(&fe_mul(&self.x, &yy, p), 4, p);
        // M = 3x^2 + a z^4
        let m = fe_add(
            &fe_mul_small(&xx, 3, p),
            &fe_mul(&c.a, &fe_sq(&zz, p), p),
            p,
        );
        let nx = fe_sub(&fe_sq(&m, p), &fe_mul_small(&s, 2, p), p);
        let ny = fe_sub(
            &fe_mul(&m, &fe_sub(&s, &nx, p), p),
            &fe_mul_small(&yyyy, 8, p),
            p,
        );
        let nz = fe_mul_small(&fe_mul(&self.y, &self.z, p), 2, p);
        JPoint {
            x: nx,
            y: ny,
            z: nz,
        }
    }

    fn add(&self, o: &JPoint, c: &EcParams) -> JPoint {
        if self.is_inf() {
            return o.clone();
        }
        if o.is_inf() {
            return self.clone();
        }
        let p = &c.p;
        let z1z1 = fe_sq(&self.z, p);
        let z2z2 = fe_sq(&o.z, p);
        let u1 = fe_mul(&self.x, &z2z2, p);
        let u2 = fe_mul(&o.x, &z1z1, p);
        let s1 = fe_mul(&self.y, &fe_mul(&o.z, &z2z2, p), p);
        let s2 = fe_mul(&o.y, &fe_mul(&self.z, &z1z1, p), p);
        let h = fe_sub(&u2, &u1, p);
        let r = fe_sub(&s2, &s1, p);
        if h.is_zero() {
            if r.is_zero() {
                return self.double(c);
            }
            return JPoint::inf();
        }
        let hh = fe_sq(&h, p);
        let hhh = fe_mul(&h, &hh, p);
        let v = fe_mul(&u1, &hh, p);
        let r2 = fe_sq(&r, p);
        let nx = fe_sub(&fe_sub(&r2, &hhh, p), &fe_mul_small(&v, 2, p), p);
        let ny = fe_sub(
            &fe_mul(&r, &fe_sub(&v, &nx, p), p),
            &fe_mul(&s1, &hhh, p),
            p,
        );
        let nz = fe_mul(&fe_mul(&self.z, &o.z, p), &h, p);
        JPoint {
            x: nx,
            y: ny,
            z: nz,
        }
    }

    fn mul(&self, k: &Big, c: &EcParams) -> JPoint {
        let mut r = JPoint::inf();
        let bits = k.bits();
        for i in (0..bits).rev() {
            r = r.double(c);
            if k.bit(i) {
                r = r.add(self, c);
            }
        }
        r
    }

    fn affine_x(&self, c: &EcParams) -> Option<Big> {
        if self.is_inf() {
            return None;
        }
        let zinv = fe_inv(&self.z, &c.p);
        let zinv2 = fe_sq(&zinv, &c.p);
        Some(fe_mul(&self.x, &zinv2, &c.p))
    }
}

/// ECDSA verify: hash is the message digest, sig = (r,s), pubkey = (qx,qy).
pub fn ecdsa_verify(c: &EcParams, qx: &Big, qy: &Big, hash: &[u8], r: &Big, s: &Big) -> bool {
    let n = &c.n;
    let one = Big::one();
    if r.cmp_big(&one) == std::cmp::Ordering::Less
        || r.cmp_big(n) != std::cmp::Ordering::Less
        || s.cmp_big(&one) == std::cmp::Ordering::Less
        || s.cmp_big(n) != std::cmp::Ordering::Less
    {
        return false;
    }
    let mut z = Big::from_be_bytes(hash);
    // Truncate hash to bit-length of n.
    let nbits = n.bits();
    let zbits = z.bits();
    for i in nbits..zbits {
        if z.bit(i) {
            z = clear_bit(&z, i);
        }
    }
    let w = s.modpow(&n.sub(&Big::from_u64(2)), n);
    let u1 = z.mul(&w).rem(n);
    let u2 = r.mul(&w).rem(n);
    let g = JPoint::affine(c.gx.clone(), c.gy.clone());
    let q = JPoint::affine(qx.clone(), qy.clone());
    let p1 = g.mul(&u1, c);
    let p2 = q.mul(&u2, c);
    let psum = p1.add(&p2, c);
    match psum.affine_x(c) {
        Some(x) => x.rem(n) == *r,
        None => false,
    }
}

fn clear_bit(b: &Big, i: usize) -> Big {
    if !b.bit(i) {
        return b.clone();
    }
    let mut d = b.d.clone();
    d[i / 64] &= !(1u64 << (i % 64));
    let mut r = Big { d };
    while r.d.last() == Some(&0) {
        r.d.pop();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x25519_rfc() {
        // RFC 7748 test vector
        let mut scalar = [0u8; 32];
        scalar[0] = 0xa5;
        scalar[1] = 0x46;
        scalar[2] = 0xe3;
        scalar[3] = 0x6b;
        scalar[4] = 0xf0;
        scalar[5] = 0x52;
        scalar[6] = 0x7c;
        scalar[7] = 0x9d;
        scalar[8] = 0x3b;
        scalar[9] = 0x16;
        scalar[10] = 0x15;
        scalar[11] = 0x4b;
        scalar[12] = 0x82;
        scalar[13] = 0x46;
        scalar[14] = 0x5e;
        scalar[15] = 0xdd;
        scalar[16] = 0x62;
        scalar[17] = 0x14;
        scalar[18] = 0x4c;
        scalar[19] = 0x0a;
        scalar[20] = 0xc1;
        scalar[21] = 0xfc;
        scalar[22] = 0x5a;
        scalar[23] = 0x18;
        scalar[24] = 0x50;
        scalar[25] = 0x6a;
        scalar[26] = 0x22;
        scalar[27] = 0x44;
        scalar[28] = 0xba;
        scalar[29] = 0x44;
        scalar[30] = 0x9a;
        scalar[31] = 0xc4;
        let mut u = [0u8; 32];
        u[0] = 0xe6;
        u[1] = 0xdb;
        u[2] = 0x68;
        u[3] = 0x67;
        u[4] = 0x58;
        u[5] = 0x30;
        u[6] = 0x30;
        u[7] = 0xdb;
        u[8] = 0x35;
        u[9] = 0x94;
        u[10] = 0xc1;
        u[11] = 0xa4;
        u[12] = 0x24;
        u[13] = 0xb1;
        u[14] = 0x5f;
        u[15] = 0x7c;
        u[16] = 0x72;
        u[17] = 0x66;
        u[18] = 0x24;
        u[19] = 0xec;
        u[20] = 0x26;
        u[21] = 0xb3;
        u[22] = 0x35;
        u[23] = 0x3b;
        u[24] = 0x10;
        u[25] = 0xa9;
        u[26] = 0x03;
        u[27] = 0xa6;
        u[28] = 0xd0;
        u[29] = 0xab;
        u[30] = 0x1c;
        u[31] = 0x4c;
        let out = x25519(&scalar, &u);
        let expect = [
            0xc3, 0xda, 0x55, 0x37, 0x9d, 0xe9, 0xc6, 0x90, 0x8e, 0x94, 0xea, 0x4d, 0xf2, 0x8d,
            0x08, 0x4f, 0x32, 0xec, 0xcf, 0x03, 0x49, 0x1c, 0x71, 0xf7, 0x54, 0xb4, 0x07, 0x55,
            0x77, 0xa2, 0x85, 0x52,
        ];
        assert_eq!(out, expect);
    }

    #[test]
    fn x25519_base() {
        let scalar = [9u8; 32];
        let mut u = [0u8; 32];
        u[0] = 9;
        let r = x25519(&scalar, &u);
        // Just ensure non-trivial output differs from input
        assert_ne!(&r[..4], &[0, 0, 0, 0]);
    }
}
