//! `hyve-validate` — Target-neutral structural validation and schema metadata reflection.

use hyve_core::{HyveError, HyveResult};
use std::ops::Deref;

/// Validation constraint attached to a model field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldConstraint {
    /// Minimum and maximum string/collection length bounds (inclusive).
    Len {
        /// Minimum allowed length.
        min: usize,
        /// Maximum allowed length.
        max: usize,
    },
    /// Minimum and maximum numeric range bounds (inclusive).
    Range {
        /// Minimum allowed numeric value.
        min: i64,
        /// Maximum allowed numeric value.
        max: i64,
    },
    /// Value must not be empty.
    NonEmpty,
}

/// Static schema metadata describing one model field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSchema {
    /// Rust field identifier (`snake_case`).
    pub name: &'static str,
    /// Rust field type string.
    pub ty: &'static str,
    /// Declared validation constraints.
    pub constraints: Vec<FieldConstraint>,
    /// True if annotated with `#[pk]`.
    pub is_pk: bool,
}

/// Static schema metadata describing a `#[model]` type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSchema {
    /// Struct identifier.
    pub name: &'static str,
    /// Optional relational table name from `#[table("...")]`.
    pub table_name: Option<&'static str>,
    /// Ordered field schemas.
    pub fields: Vec<FieldSchema>,
}

/// Trait implemented automatically by `#[model]` types exposing schema metadata.
pub trait HasSchema {
    /// Returns static schema metadata for this model.
    fn schema() -> ModelSchema;
}

/// Structured field-level validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Failing field name.
    pub field: &'static str,
    /// Machine-readable constraint violation code.
    pub code: &'static str,
    /// Human-readable description with fix guidance.
    pub message: String,
}

/// Trait implemented by models supporting structural validation.
pub trait Validate {
    /// Validates `self`, returning structured field errors if any invariant fails.
    fn validate(&self) -> Result<(), Vec<ValidationError>>;
}

/// Newtype wrapper guaranteeing the inner `T` passed all [`Validate`] rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Valid<T>(T);

impl<T: Validate> Valid<T> {
    /// Validates `val`, returning `Err(HyveError::domain("invalid_request", ...))` on failure.
    pub fn new(val: T) -> HyveResult<Self> {
        match val.validate() {
            Ok(()) => Ok(Self(val)),
            Err(errors) => {
                let summary = errors
                    .iter()
                    .map(|e| format!("{}: {}", e.field, e.message))
                    .collect::<Vec<_>>()
                    .join("; ");
                Err(HyveError::domain("invalid_request", summary))
            }
        }
    }

    /// Consumes the wrapper and returns the validated inner value.
    #[must_use]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> Deref for Valid<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Helper trait used by `#[model]` to measure string/collection length cleanly.
pub trait ValidateLen {
    /// Returns the length used for `#[validate(len = ...)]`.
    fn validate_len(&self) -> usize;
}

impl ValidateLen for String {
    fn validate_len(&self) -> usize {
        self.chars().count()
    }
}

impl ValidateLen for str {
    fn validate_len(&self) -> usize {
        self.chars().count()
    }
}

impl<T> ValidateLen for Vec<T> {
    fn validate_len(&self) -> usize {
        self.len()
    }
}

/// Helper trait used by `#[model]` to evaluate `#[validate(range = ...)]` uniformly across integers.
pub trait ValidateRange {
    /// Converts `self` to `i128` for inclusive range checking.
    fn validate_as_i128(&self) -> i128;
}

macro_rules! impl_validate_range_int {
    ($($ty:ty),*) => {
        $(
            impl ValidateRange for $ty {
                fn validate_as_i128(&self) -> i128 {
                    i128::from(*self)
                }
            }
        )*
    };
}

impl_validate_range_int!(u8, u16, u32, u64, i8, i16, i32, i64, i128);

impl ValidateRange for usize {
    fn validate_as_i128(&self) -> i128 {
        i128::try_from(*self).unwrap_or(i128::MAX)
    }
}

/// Helper called by generated `#[model]` code to validate `#[validate(len = min..=max)]`.
pub fn check_len<V: ValidateLen>(
    field: &'static str,
    val: &V,
    min: usize,
    max: usize,
    errors: &mut Vec<ValidationError>,
) {
    let actual = val.validate_len();
    if actual < min || actual > max {
        errors.push(ValidationError {
            field,
            code: "len_out_of_range",
            message: format!("length {actual} must be within {min}..={max}"),
        });
    }
}

/// Helper called by generated `#[model]` code to validate `#[validate(range = min..=max)]`.
pub fn check_range<V: ValidateRange>(
    field: &'static str,
    val: &V,
    min: i64,
    max: i64,
    errors: &mut Vec<ValidationError>,
) {
    let actual = val.validate_as_i128();
    if actual < i128::from(min) || actual > i128::from(max) {
        errors.push(ValidationError {
            field,
            code: "value_out_of_range",
            message: format!("value {actual} must be within {min}..={max}"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Dummy {
        name: String,
        fps: u32,
    }

    impl Validate for Dummy {
        fn validate(&self) -> Result<(), Vec<ValidationError>> {
            let mut errs = Vec::new();
            check_len("name", &self.name, 1, 80, &mut errs);
            check_range("fps", &self.fps, 1, 240, &mut errs);
            if errs.is_empty() { Ok(()) } else { Err(errs) }
        }
    }

    #[test]
    fn valid_wrapper_accepts_good_and_rejects_bad_inputs() {
        assert!(
            Valid::new(Dummy {
                name: "pbr".into(),
                fps: 120
            })
            .is_ok()
        );

        let err = Valid::new(Dummy {
            name: String::new(),
            fps: 500,
        })
        .unwrap_err();
        assert_eq!(err.code(), "invalid_request");
        assert_eq!(err.http_status(), 400);
    }
}
