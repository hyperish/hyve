//! Sortable identifiers.
//!
//! [`Ulid`] follows the ULID specification (48-bit millisecond timestamp + 80
//! random bits, Crockford base32): <https://github.com/ulid/spec>. The random
//! part comes from the platform CSPRNG and, within one process, consecutive IDs
//! are strictly increasing (the spec's monotonic mode). IDs identify things; they
//! are not credentials (`hyperish/docs` `02-core-contracts.md` §6).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Mutex;

use crate::platform;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const RANDOM_BITS: u32 = 80;
const RANDOM_MASK: u128 = (1u128 << RANDOM_BITS) - 1;
const MAX_MILLIS: u64 = (1u64 << 48) - 1;

/// State of the process-wide monotonic generator: (last millis, last random).
static LAST: Mutex<Option<(u64, u128)>> = Mutex::new(None);

/// A 128-bit ULID.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[rkyv(derive(Debug, PartialEq, Eq))]
pub struct Ulid(u128);

impl Ulid {
    /// Generates a new ULID from the current time and the CSPRNG.
    ///
    /// Within one process the result is strictly greater than every ULID this
    /// function returned before, even when called many times in the same
    /// millisecond or when the wall clock steps backwards. If the 80-bit random
    /// part would overflow inside one millisecond, the timestamp part advances by
    /// one millisecond instead of failing.
    ///
    /// # Panics
    ///
    /// Panics if the platform CSPRNG is unavailable.
    pub fn new() -> Self {
        let now = (platform::now_unix_micros().max(0) / 1000) as u64;
        let now = now.min(MAX_MILLIS);
        let mut last = LAST.lock().unwrap_or_else(|p| p.into_inner());
        let (millis, random) = match *last {
            Some((last_ms, last_rand)) if now <= last_ms => {
                if last_rand < RANDOM_MASK {
                    (last_ms, last_rand + 1)
                } else {
                    (last_ms + 1, fresh_random())
                }
            }
            _ => (now, fresh_random()),
        };
        *last = Some((millis, random));
        Ulid::from_parts(millis, random)
    }

    /// Builds a ULID from a 48-bit millisecond timestamp and 80 random bits
    /// (higher bits are discarded).
    pub const fn from_parts(millis: u64, random: u128) -> Self {
        Ulid(((millis as u128 & MAX_MILLIS as u128) << RANDOM_BITS) | (random & RANDOM_MASK))
    }

    /// The raw 128-bit value.
    pub const fn to_u128(self) -> u128 {
        self.0
    }

    /// Builds a ULID from its raw 128-bit value.
    pub const fn from_u128(v: u128) -> Self {
        Ulid(v)
    }

    /// Millisecond timestamp part.
    pub const fn millis(self) -> u64 {
        (self.0 >> RANDOM_BITS) as u64
    }

    /// The 26-character Crockford base32 form.
    pub fn encode(self) -> [u8; 26] {
        let mut out = [0u8; 26];
        let mut v = self.0;
        for slot in out.iter_mut().rev() {
            *slot = ALPHABET[(v & 31) as usize];
            v >>= 5;
        }
        out
    }
}

impl Default for Ulid {
    fn default() -> Self {
        Ulid::new()
    }
}

fn fresh_random() -> u128 {
    let mut bytes = [0u8; 10];
    platform::fill_random(&mut bytes);
    let mut buf = [0u8; 16];
    buf[6..].copy_from_slice(&bytes);
    u128::from_be_bytes(buf)
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let enc = self.encode();
        // ALPHABET is ASCII, so this never fails.
        f.write_str(std::str::from_utf8(&enc).map_err(|_| fmt::Error)?)
    }
}

impl fmt::Debug for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ulid({self})")
    }
}

/// Error returned when text is not a valid ULID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseUlidError(&'static str);

impl fmt::Display for ParseUlidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid ULID: {}", self.0)
    }
}

impl std::error::Error for ParseUlidError {}

impl FromStr for Ulid {
    type Err = ParseUlidError;

    /// Parses the 26-character form, case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 26 {
            return Err(ParseUlidError("expected 26 characters"));
        }
        let mut v: u128 = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let d = decode_char(b).ok_or(ParseUlidError("invalid character"))?;
            if i == 0 && d > 7 {
                return Err(ParseUlidError("value exceeds 128 bits"));
            }
            v = (v << 5) | d as u128;
        }
        Ok(Ulid(v))
    }
}

const INVALID: u8 = 0xFF;

/// The digit each byte stands for, upper or lower case; `INVALID` for the
/// rest.
const DECODE: [u8; 256] = {
    let mut table = [INVALID; 256];
    let mut i = 0;
    while i < ALPHABET.len() {
        table[ALPHABET[i] as usize] = i as u8;
        table[ALPHABET[i].to_ascii_lowercase() as usize] = i as u8;
        i += 1;
    }
    table
};

fn decode_char(b: u8) -> Option<u8> {
    match DECODE[b as usize] {
        INVALID => None,
        d => Some(d),
    }
}

/// A [`Ulid`] tagged with the type it identifies, so `Id<User>` and
/// `Id<Project>` cannot be mixed up.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Id<T: ?Sized> {
    ulid: Ulid,
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Id<T> {
    /// Generates a new ID (see [`Ulid::new`]).
    pub fn new() -> Self {
        Self::from_ulid(Ulid::new())
    }

    /// Wraps an existing ULID.
    pub const fn from_ulid(ulid: Ulid) -> Self {
        Id {
            ulid,
            _marker: PhantomData,
        }
    }

    /// The underlying ULID.
    pub const fn ulid(self) -> Ulid {
        self.ulid
    }

    /// Reinterprets the ID as identifying another type.
    pub const fn cast<U: ?Sized>(self) -> Id<U> {
        Id::from_ulid(self.ulid)
    }
}

impl<T: ?Sized> Default for Id<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Id<T> {}

impl<T: ?Sized> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ulid == other.ulid
    }
}
impl<T: ?Sized> Eq for Id<T> {}

impl<T: ?Sized> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: ?Sized> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ulid.cmp(&other.ulid)
    }
}

impl<T: ?Sized> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ulid.hash(state);
    }
}

impl<T: ?Sized> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.ulid, f)
    }
}

impl<T: ?Sized> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self.ulid)
    }
}

impl<T: ?Sized> FromStr for Id<T> {
    type Err = ParseUlidError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self::from_ulid)
    }
}

impl From<&ArchivedUlid> for Ulid {
    fn from(a: &ArchivedUlid) -> Self {
        Ulid(a.0.to_native())
    }
}

impl<T: ?Sized> From<&ArchivedId<T>> for Id<T> {
    fn from(a: &ArchivedId<T>) -> Self {
        Id::from_ulid(Ulid::from(&a.ulid))
    }
}

impl<T: ?Sized> From<Ulid> for Id<T> {
    fn from(u: Ulid) -> Self {
        Self::from_ulid(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_text() {
        for _ in 0..100 {
            let u = Ulid::new();
            let s = u.to_string();
            assert_eq!(s.len(), 26);
            assert_eq!(s.parse::<Ulid>().unwrap(), u);
            assert_eq!(s.to_ascii_lowercase().parse::<Ulid>().unwrap(), u);
        }
    }

    #[test]
    fn spec_example_layout() {
        // Max value from the spec.
        let max: Ulid = "7ZZZZZZZZZZZZZZZZZZZZZZZZZ".parse().unwrap();
        assert_eq!(max.to_u128(), u128::MAX);
        assert!("8ZZZZZZZZZZZZZZZZZZZZZZZZZ".parse::<Ulid>().is_err());
        assert!("01ARZ3NDEKTSV4RRFFQ69G5FAI".parse::<Ulid>().is_err()); // I excluded
        let u = Ulid::from_parts(1_469_918_176_385, 0);
        assert!(u.to_string().starts_with("01ARYZ6S41"));
    }

    #[test]
    fn every_byte_decodes_as_the_alphabet_says() {
        for b in 0..=u8::MAX {
            let by_search = ALPHABET
                .iter()
                .position(|&a| a == b.to_ascii_uppercase())
                .map(|p| p as u8);
            assert_eq!(decode_char(b), by_search, "byte {b:#04x}");
        }
    }

    #[test]
    fn strictly_monotonic_in_process() {
        let mut prev = Ulid::new();
        for _ in 0..10_000 {
            let next = Ulid::new();
            assert!(next > prev);
            prev = next;
        }
    }

    #[test]
    fn random_part_is_not_constant() {
        let a = Ulid::from_parts(0, fresh_random());
        let b = Ulid::from_parts(0, fresh_random());
        assert_ne!(a, b);
    }

    #[test]
    fn typed_ids_are_copy_and_comparable() {
        struct User;
        let a: Id<User> = Id::new();
        let b = a;
        assert_eq!(a, b);
        assert_eq!(a.to_string().parse::<Id<User>>().unwrap(), a);
    }
}
