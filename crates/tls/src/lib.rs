//! sol_tls: a from-scratch TLS 1.3 client — no third-party crypto.
//!
//! Components: SHA-256/384/512 + HMAC + HKDF (`sha2`), big-integer
//! arithmetic (`bignum`), X25519 + ECDSA P-256/P-384 (`curve`), AES-GCM
//! and ChaCha20-Poly1305 AEAD (`aead`), ASN.1 DER (`der`), X.509 chain
//! verification (`x509`), and the handshake/record layer (`hs`).

pub mod aead;
pub mod bignum;
pub mod curve;
pub mod der;
mod hs;
mod rng;
pub mod sha2;
pub mod x509;

pub use hs::TlsStream;
