//! Compile-time schema fingerprints.
//!
//! A fingerprint covers a type's wire layout (field names, order and field
//! types, recursively) and [`FORMAT_FINGERPRINT`] (the binary codec version
//! and format control). Two builds may use the fast rail with each other only
//! when the fingerprints of the request, response and error types match
//! (`docs/03-adapters-and-pipeline.md` §3).

use std::collections::{BTreeMap, HashMap};

use hyve_core::{Id, Timestamp, Ulid, Value};

/// FNV-1a over `bytes` (usable in constants).
pub const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
        i += 1;
    }
    h
}

/// Order-sensitive combination of two fingerprints.
pub const fn mix(a: u64, b: u64) -> u64 {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let mut buf = [0u8; 16];
    let mut i = 0;
    while i < 8 {
        buf[i] = a[i];
        buf[8 + i] = b[i];
        i += 1;
    }
    fnv1a(&buf)
}

/// Codec format identity mixed into every model fingerprint. Changes whenever
/// the pinned `rkyv` version or the format control changes.
pub const FORMAT_FINGERPRINT: u64 =
    fnv1a(b"rkyv=0.8.18;little_endian;aligned;pointer_width_32;json=hyve-1");

/// A type's compile-time wire-layout fingerprint.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a wire type (it has no schema fingerprint)",
    label = "not a wire type",
    note = "declare it with `#[model]`, or use a supported type (integers except `usize`/`isize`, floats, `bool`, `char`, `String`, `Ulid`, `Id<T>`, `Timestamp`, `Value`, `Option`, `Vec`, `Box`, arrays, `BTreeMap<String, _>`, `HashMap<String, _>`)"
)]
pub trait Fingerprint {
    /// The fingerprint.
    const FINGERPRINT: u64;
}

macro_rules! leaf {
    ($($t:ty => $name:literal),* $(,)?) => {$(
        impl Fingerprint for $t {
            const FINGERPRINT: u64 = fnv1a($name.as_bytes());
        }
    )*};
}

leaf! {
    () => "()", bool => "bool", char => "char",
    i8 => "i8", i16 => "i16", i32 => "i32", i64 => "i64", i128 => "i128",
    u8 => "u8", u16 => "u16", u32 => "u32", u64 => "u64", u128 => "u128",
    f32 => "f32", f64 => "f64", String => "String",
    Ulid => "Ulid", Timestamp => "Timestamp", Value => "Value",
}

impl<T: ?Sized> Fingerprint for Id<T> {
    const FINGERPRINT: u64 = fnv1a(b"Ulid");
}

impl<T: Fingerprint> Fingerprint for Option<T> {
    const FINGERPRINT: u64 = mix(fnv1a(b"Option"), T::FINGERPRINT);
}

impl<T: Fingerprint> Fingerprint for Vec<T> {
    const FINGERPRINT: u64 = mix(fnv1a(b"Vec"), T::FINGERPRINT);
}

impl<T: Fingerprint> Fingerprint for Box<T> {
    const FINGERPRINT: u64 = mix(fnv1a(b"Box"), T::FINGERPRINT);
}

impl<T: Fingerprint, const N: usize> Fingerprint for [T; N] {
    const FINGERPRINT: u64 = mix(mix(fnv1a(b"Array"), N as u64), T::FINGERPRINT);
}

impl<V: Fingerprint> Fingerprint for BTreeMap<String, V> {
    const FINGERPRINT: u64 = mix(fnv1a(b"BTreeMap<String>"), V::FINGERPRINT);
}

impl<V: Fingerprint, S> Fingerprint for HashMap<String, V, S> {
    const FINGERPRINT: u64 = mix(fnv1a(b"HashMap<String>"), V::FINGERPRINT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_and_order_sensitive() {
        assert_ne!(<Vec<u32>>::FINGERPRINT, <Vec<u64>>::FINGERPRINT);
        assert_ne!(
            <Option<Vec<u8>>>::FINGERPRINT,
            <Vec<Option<u8>>>::FINGERPRINT
        );
        assert_ne!(mix(1, 2), mix(2, 1));
        struct A;
        struct B;
        assert_eq!(<Id<A>>::FINGERPRINT, <Id<B>>::FINGERPRINT);
        assert_eq!(<Id<A>>::FINGERPRINT, Ulid::FINGERPRINT);
    }
}
