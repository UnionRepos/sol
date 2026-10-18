//! Entropy from the OS CSPRNG (/dev/urandom).

use std::io::Read;

pub fn rand_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    let mut f = std::fs::File::open("/dev/urandom").expect("open urandom");
    f.read_exact(&mut v).expect("read urandom");
    v
}

pub fn rand_array<const N: usize>() -> [u8; N] {
    let mut v = [0u8; N];
    let mut f = std::fs::File::open("/dev/urandom").expect("open urandom");
    f.read_exact(&mut v).expect("read urandom");
    v
}
