//! Minimal arbitrary-precision unsigned integer for RSA/ECDSA verification.
//! Little-endian u64 limbs. Implements schoolbook mul, Knuth D division,
//! and square-multiply modpow. Performance is adequate: used once per
//! connection for a handful of signature verifications.

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Big {
    pub d: Vec<u64>,
}

impl Big {
    pub fn zero() -> Big {
        Big { d: vec![] }
    }
    pub fn one() -> Big {
        Big { d: vec![1] }
    }
    pub fn from_u64(v: u64) -> Big {
        if v == 0 {
            Big::zero()
        } else {
            Big { d: vec![v] }
        }
    }
    pub fn from_be_bytes(b: &[u8]) -> Big {
        // pack bytes into limbs
        let mut d = Vec::with_capacity(b.len() / 8 + 1);
        let mut i = b.len();
        while i >= 8 {
            d.push(u64::from_be_bytes([
                b[i - 8],
                b[i - 7],
                b[i - 6],
                b[i - 5],
                b[i - 4],
                b[i - 3],
                b[i - 2],
                b[i - 1],
            ]));
            i -= 8;
        }
        if i > 0 {
            let mut limb = 0u64;
            for &x in &b[..i] {
                limb = (limb << 8) | x as u64;
            }
            d.push(limb);
        }
        let mut r = Big { d };
        r.norm();
        r
    }
    pub fn to_be_bytes(&self, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        for (i, &limb) in self.d.iter().enumerate() {
            for j in 0..8 {
                let byte_idx = len as i64 - 1 - (i * 8 + j) as i64;
                if byte_idx >= 0 {
                    out[byte_idx as usize] = (limb >> (j * 8)) as u8;
                }
            }
        }
        out
    }
    fn norm(&mut self) {
        while self.d.last() == Some(&0) {
            self.d.pop();
        }
    }
    pub fn is_zero(&self) -> bool {
        self.d.is_empty()
    }
    pub fn is_even(&self) -> bool {
        self.d.first().map(|x| x & 1 == 0).unwrap_or(true)
    }
    pub fn bits(&self) -> usize {
        match self.d.last() {
            None => 0,
            Some(&t) => self.d.len() * 64 - t.leading_zeros() as usize,
        }
    }
    pub fn bit(&self, i: usize) -> bool {
        self.d
            .get(i / 64)
            .map(|x| (x >> (i % 64)) & 1 == 1)
            .unwrap_or(false)
    }
    pub fn cmp_big(&self, o: &Big) -> Ordering {
        if self.d.len() != o.d.len() {
            return self.d.len().cmp(&o.d.len());
        }
        for i in (0..self.d.len()).rev() {
            if self.d[i] != o.d[i] {
                return self.d[i].cmp(&o.d[i]);
            }
        }
        Ordering::Equal
    }
    pub fn add(&self, o: &Big) -> Big {
        let mut d = Vec::with_capacity(self.d.len().max(o.d.len()) + 1);
        let mut carry = 0u64;
        for i in 0..self.d.len().max(o.d.len()) {
            let a = self.d.get(i).copied().unwrap_or(0);
            let b = o.d.get(i).copied().unwrap_or(0);
            let (s1, c1) = a.overflowing_add(b);
            let (s2, c2) = s1.overflowing_add(carry);
            d.push(s2);
            carry = (c1 | c2) as u64;
        }
        if carry > 0 {
            d.push(carry);
        }
        let mut r = Big { d };
        r.norm();
        r
    }
    /// self - o; requires self >= o.
    pub fn sub(&self, o: &Big) -> Big {
        let mut d = Vec::with_capacity(self.d.len());
        let mut borrow = false;
        for i in 0..self.d.len() {
            let a = self.d[i];
            let b = o.d.get(i).copied().unwrap_or(0);
            let (s1, b1) = a.overflowing_sub(b);
            let (s2, b2) = s1.overflowing_sub(borrow as u64);
            d.push(s2);
            borrow = b1 | b2;
        }
        let mut r = Big { d };
        r.norm();
        r
    }
    /// Modular subtract: (a - b) mod m for a,b < m.
    pub fn sub_mod(&self, o: &Big, m: &Big) -> Big {
        if self.cmp_big(o) != Ordering::Less {
            self.sub(o)
        } else {
            // self + m - o
            self.add(m).sub(o)
        }
    }
    pub fn mul(&self, o: &Big) -> Big {
        if self.is_zero() || o.is_zero() {
            return Big::zero();
        }
        let mut d = vec![0u64; self.d.len() + o.d.len()];
        for (i, &a) in self.d.iter().enumerate() {
            let mut carry = 0u128;
            for (j, &b) in o.d.iter().enumerate() {
                let cur = d[i + j] as u128 + a as u128 * b as u128 + carry;
                d[i + j] = cur as u64;
                carry = cur >> 64;
            }
            let mut k = i + o.d.len();
            while carry > 0 {
                let cur = d[k] as u128 + carry;
                d[k] = cur as u64;
                carry = cur >> 64;
                k += 1;
            }
        }
        let mut r = Big { d };
        r.norm();
        r
    }
    pub fn shl1(&self) -> Big {
        let mut d = Vec::with_capacity(self.d.len() + 1);
        let mut carry = 0u64;
        for &x in &self.d {
            d.push((x << 1) | carry);
            carry = x >> 63;
        }
        if carry > 0 {
            d.push(carry);
        }
        let mut r = Big { d };
        r.norm();
        r
    }
    pub fn shr1(&self) -> Big {
        let mut d = vec![0u64; self.d.len()];
        let mut carry = 0u64;
        for i in (0..self.d.len()).rev() {
            d[i] = (self.d[i] >> 1) | carry;
            carry = self.d[i] << 63;
        }
        let mut r = Big { d };
        r.norm();
        r
    }

    /// Knuth Algorithm D long division: returns (quotient, remainder).
    pub fn divmod(&self, o: &Big) -> (Big, Big) {
        if o.is_zero() {
            panic!("division by zero");
        }
        if self.cmp_big(o) == Ordering::Less {
            return (Big::zero(), self.clone());
        }
        if o.d.len() == 1 {
            // short division
            let dv = o.d[0];
            let mut q = vec![0u64; self.d.len()];
            let mut rem = 0u128;
            for i in (0..self.d.len()).rev() {
                let cur = (rem << 64) | self.d[i] as u128;
                q[i] = (cur / dv as u128) as u64;
                rem = cur % dv as u128;
            }
            let mut qq = Big { d: q };
            qq.norm();
            return (qq, Big::from_u64(rem as u64));
        }
        // Normalize so top bit of divisor is set.
        let shift = o.d.last().unwrap().leading_zeros() as usize;
        let v = shl_bits(o, shift);
        let u = shl_bits(self, shift);
        let n = v.d.len();
        let m = u.d.len() - n;
        let mut uu = u.d.clone();
        uu.resize(u.d.len().max(m + n) + 1, 0);
        let mut q = vec![0u64; m + 1];
        for j in (0..=m).rev() {
            let num = ((uu[j + n] as u128) << 64) | uu[j + n - 1] as u128;
            let mut qhat = num / v.d[n - 1] as u128;
            let mut rhat = num % v.d[n - 1] as u128;
            loop {
                if qhat >= (1u128 << 64)
                    || qhat * v.d[n - 2] as u128 > ((rhat << 64) | uu[j + n - 2] as u128)
                {
                    qhat -= 1;
                    rhat += v.d[n - 1] as u128;
                    if rhat < (1u128 << 64) {
                        continue;
                    }
                }
                break;
            }
            // multiply-subtract
            let mut borrow: i128 = 0;
            let mut carry: u128 = 0;
            for i in 0..n {
                let p = qhat * v.d[i] as u128 + carry;
                carry = p >> 64;
                let sub = uu[i + j] as i128 - (p as u64) as i128 + borrow;
                if sub < 0 {
                    uu[i + j] = (sub + (1i128 << 64)) as u64;
                    borrow = -1;
                } else {
                    uu[i + j] = sub as u64;
                    borrow = 0;
                }
            }
            let sub = uu[j + n] as i128 - carry as i128 + borrow;
            if sub < 0 {
                // add back
                qhat -= 1;
                let mut c = 0u128;
                for i in 0..n {
                    let s = uu[i + j] as u128 + v.d[i] as u128 + c;
                    uu[i + j] = s as u64;
                    c = s >> 64;
                }
                uu[j + n] = (uu[j + n] as u128 + c) as u64;
            } else {
                uu[j + n] = sub as u64;
            }
            q[j] = qhat as u64;
        }
        let mut qq = Big { d: q };
        qq.norm();
        let rr = Big {
            d: uu[..n].to_vec(),
        };
        let mut r = shr_bits(&rr, shift);
        r.norm();
        (qq, r)
    }

    pub fn rem(&self, m: &Big) -> Big {
        self.divmod(m).1
    }

    /// (self ^ e) mod m, square-and-multiply.
    pub fn modpow(&self, e: &Big, m: &Big) -> Big {
        if m.d == vec![1] {
            return Big::zero();
        }
        let mut base = self.rem(m);
        let mut result = Big::one();
        let bits = e.bits();
        for i in 0..bits {
            if e.bit(i) {
                result = result.mul(&base).rem(m);
            }
            if i + 1 < bits {
                base = base.mul(&base).rem(m);
            }
        }
        result
    }
}

fn shl_bits(a: &Big, s: usize) -> Big {
    if s == 0 {
        return a.clone();
    }
    let limb_shift = s / 64;
    let bit_shift = s % 64;
    let mut d = vec![0u64; limb_shift];
    let mut carry = 0u64;
    for &x in &a.d {
        if bit_shift == 0 {
            d.push(x);
        } else {
            d.push((x << bit_shift) | carry);
            carry = x >> (64 - bit_shift);
        }
    }
    if carry > 0 {
        d.push(carry);
    }
    let mut r = Big { d };
    r.norm();
    r
}

fn shr_bits(a: &Big, s: usize) -> Big {
    if s == 0 {
        return a.clone();
    }
    let limb_shift = s / 64;
    let bit_shift = s % 64;
    let mut d = Vec::new();
    for i in limb_shift..a.d.len() {
        let mut v = a.d[i] >> bit_shift;
        if bit_shift > 0 && i + 1 < a.d.len() {
            v |= a.d[i + 1] << (64 - bit_shift);
        }
        d.push(v);
    }
    let mut r = Big { d };
    r.norm();
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: u64) -> Big {
        Big::from_u64(v)
    }

    #[test]
    fn addsub() {
        let a = Big::from_be_bytes(&[0xff; 20]);
        let o = Big::one();
        let s = a.add(&o).sub(&a);
        assert_eq!(s, o);
    }

    #[test]
    fn muldiv() {
        let a = b(0xffffffffffffffff);
        let c = a.mul(&a);
        let (q, r) = c.divmod(&a);
        assert_eq!(q, a);
        assert!(r.is_zero());
    }

    #[test]
    fn divmod_random() {
        // (a*b + r) / b == a, r
        let a = Big::from_be_bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
        let m = Big::from_be_bytes(&[9, 8, 7, 6, 5, 4, 3, 2, 1]);
        let r = Big::from_u64(12345);
        let n = a.mul(&m).add(&r);
        let (q, rr) = n.divmod(&m);
        assert_eq!(q, a);
        assert_eq!(rr, r);
    }

    #[test]
    fn modpow() {
        // 3^1000 mod 7919 = ? compute via u128 to cross-check small case:
        let base = b(3);
        let e = b(1000);
        let m = b(7919);
        let r = base.modpow(&e, &m);
        let mut expect = 1u128;
        for _ in 0..1000 {
            expect = expect * 3 % 7919;
        }
        assert_eq!(r, b(expect as u64));
    }

    #[test]
    fn rsa_sized_modpow() {
        // 2048-bit-ish quick sanity: (a^e mod m) runs and is < m
        let a = Big::from_be_bytes(&[0xab; 256]);
        let e = Big::from_u64(65537);
        let m = Big::from_be_bytes(&[0xff; 256]);
        let r = a.modpow(&e, &m);
        assert!(r.cmp_big(&m) == Ordering::Less);
    }
}
