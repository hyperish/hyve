//! Binary (fast-rail) codec on top of `rkyv`.

use std::num::NonZeroUsize;
use std::ops::Deref;

use rkyv::api::high::{HighDeserializer, HighSerializer, HighValidator};
use rkyv::bytecheck::CheckBytes;
use rkyv::rancor::Error as RkyvError;
use rkyv::ser::allocator::ArenaHandle;
use rkyv::util::AlignedVec;
use rkyv::validation::Validator;
use rkyv::validation::archive::ArchiveValidator;
use rkyv::validation::shared::SharedValidator;
use rkyv::{Archive, Deserialize, Portable, Serialize};

use crate::{DecodeError, EncodeError};

/// Deepest nesting of out-of-line data (`Vec`, `String`, `Box`, nested
/// `Value`s, …) accepted from untrusted bytes; the same bound as the JSON
/// reader's default depth. Validation of deeper archives fails instead of
/// exhausting the stack.
pub const MAX_BINARY_DEPTH: usize = 128;

/// Alignment of encoded buffers (`rkyv::util::AlignedVec`'s default).
const ALIGN: usize = 16;

/// Encoded bytes, aligned for zero-copy access.
#[derive(Debug, Default, Clone)]
pub struct AlignedBytes(AlignedVec<ALIGN>);

impl AlignedBytes {
    /// Copies `bytes` into an aligned buffer.
    pub fn copy_from(bytes: &[u8]) -> Self {
        let mut v = AlignedVec::<ALIGN>::with_capacity(bytes.len());
        v.extend_from_slice(bytes);
        AlignedBytes(v)
    }

    /// Moves the bytes out (one copy into a plain `Vec`).
    pub fn into_vec(self) -> Vec<u8> {
        self.0.into_vec()
    }
}

impl Deref for AlignedBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl PartialEq for AlignedBytes {
    fn eq(&self, other: &Self) -> bool {
        self[..] == other[..]
    }
}

impl Eq for AlignedBytes {}

impl AsRef<[u8]> for AlignedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Types that travel on the fast rail. Implemented for every type whose `rkyv`
/// archive can be serialized, validated and deserialized (`#[model]` derives
/// what is needed); use it as the single bound instead of spelling out `rkyv`'s.
pub trait Binary: Sized {
    /// Encodes `self`.
    fn to_binary(&self) -> Result<AlignedBytes, EncodeError>;

    /// Validates and decodes `bytes` (copied into an aligned buffer first only
    /// when they are misaligned).
    fn from_binary(bytes: &[u8]) -> Result<Self, DecodeError>;
}

impl<T> Binary for T
where
    T: Archive + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>>,
    T::Archived: for<'a> CheckBytes<HighValidator<'a, RkyvError>>
        + Deserialize<T, HighDeserializer<RkyvError>>,
{
    fn to_binary(&self) -> Result<AlignedBytes, EncodeError> {
        rkyv::to_bytes::<RkyvError>(self)
            .map(AlignedBytes)
            .map_err(|e| EncodeError(e.to_string()))
    }

    fn from_binary(bytes: &[u8]) -> Result<Self, DecodeError> {
        with_aligned(bytes, |b| {
            let archived = checked_access::<T::Archived>(b)?;
            rkyv::deserialize::<T, RkyvError>(archived)
        })
        .map_err(|e| DecodeError::new(format!("invalid binary payload: {e}"), "$", None))
    }
}

/// Validated zero-copy access to an archived `T` inside `bytes`, which must be
/// aligned (as [`AlignedBytes`] are).
pub fn access<T>(bytes: &[u8]) -> Result<&T::Archived, DecodeError>
where
    T: Archive,
    T::Archived: Portable + for<'a> CheckBytes<HighValidator<'a, RkyvError>>,
{
    checked_access::<T::Archived>(bytes)
        .map_err(|e| DecodeError::new(format!("invalid binary payload: {e}"), "$", None))
}

/// `rkyv`'s high-level validator, but with a depth bound ([`MAX_BINARY_DEPTH`]);
/// the high-level API validates without one.
fn checked_access<'a, A>(bytes: &'a [u8]) -> Result<&'a A, RkyvError>
where
    A: Portable + CheckBytes<HighValidator<'a, RkyvError>>,
{
    let mut validator = Validator::new(
        ArchiveValidator::with_max_depth(bytes, NonZeroUsize::new(MAX_BINARY_DEPTH)),
        SharedValidator::new(),
    );
    rkyv::api::access_with_context::<A, _, RkyvError>(bytes, &mut validator)
}

fn with_aligned<R>(bytes: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
    if (bytes.as_ptr() as usize) % ALIGN == 0 {
        f(bytes)
    } else {
        let copy = AlignedBytes::copy_from(bytes);
        f(&copy)
    }
}
