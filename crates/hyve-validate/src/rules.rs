//! Built-in rules used by `#[validate(...)]`.
//!
//! - `len = <range>`: length of a string (in Unicode scalar values), list,
//!   array or map. `Option` fields are checked only when `Some`.
//! - `range = <range>`: numeric bounds. `Option` fields are checked only when
//!   `Some`; `NaN` never satisfies a float range.
//!
//! Ranges are ordinary Rust ranges (`1..=80`, `1..80`, `..=80`, `1..`).

use std::collections::{BTreeMap, HashMap};
use std::marker::PhantomData;
use std::ops::{Bound, RangeBounds};

use hyve_core::Value;

use crate::{Constraint, FieldPath, Violation, Violations};

/// Things with a length for the `len` rule.
#[diagnostic::on_unimplemented(
    message = "`#[validate(len = ...)]` does not apply to `{Self}`",
    label = "this field has no length",
    note = "`len` checks strings, lists, arrays and maps (or an `Option` of one); use `range` for numbers"
)]
pub trait HasLen {
    /// The length, or `None` to skip the rule (an absent `Option`).
    fn rule_len(&self) -> Option<usize>;
    /// Unit used in messages.
    fn len_unit(&self) -> &'static str {
        "items"
    }
}

impl HasLen for String {
    fn rule_len(&self) -> Option<usize> {
        Some(self.chars().count())
    }
    fn len_unit(&self) -> &'static str {
        "characters"
    }
}

impl HasLen for str {
    fn rule_len(&self) -> Option<usize> {
        Some(self.chars().count())
    }
    fn len_unit(&self) -> &'static str {
        "characters"
    }
}

impl<T> HasLen for Vec<T> {
    fn rule_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl<T> HasLen for [T] {
    fn rule_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl<T, const N: usize> HasLen for [T; N] {
    fn rule_len(&self) -> Option<usize> {
        Some(N)
    }
}

impl<K, V> HasLen for BTreeMap<K, V> {
    fn rule_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl<K, V, S> HasLen for HashMap<K, V, S> {
    fn rule_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

impl<T: HasLen> HasLen for Option<T> {
    fn rule_len(&self) -> Option<usize> {
        self.as_ref().and_then(HasLen::rule_len)
    }
    fn len_unit(&self) -> &'static str {
        self.as_ref().map_or("items", HasLen::len_unit)
    }
}

impl<T: HasLen + ?Sized> HasLen for Box<T> {
    fn rule_len(&self) -> Option<usize> {
        (**self).rule_len()
    }
    fn len_unit(&self) -> &'static str {
        (**self).len_unit()
    }
}

/// Inclusive `(min, max)` of a length range (for messages and schemas).
pub fn len_bounds<R: RangeBounds<usize>>(range: &R) -> (Option<u64>, Option<u64>) {
    let min = match range.start_bound() {
        Bound::Included(&n) => Some(n as u64),
        Bound::Excluded(&n) => Some(n as u64 + 1),
        Bound::Unbounded => None,
    };
    let max = match range.end_bound() {
        Bound::Included(&n) => Some(n as u64),
        Bound::Excluded(&n) => Some((n as u64).saturating_sub(1)),
        Bound::Unbounded => None,
    };
    (min, max)
}

/// The `len` rule.
pub fn len<V: HasLen + ?Sized, R: RangeBounds<usize>>(
    value: &V,
    range: R,
    path: &FieldPath,
    out: &mut Violations,
) {
    let Some(n) = value.rule_len() else { return };
    if range.contains(&n) {
        return;
    }
    let (min, max) = len_bounds(&range);
    let unit = value.len_unit();
    let message = match (min, max) {
        (Some(a), Some(b)) if a == b => format!("must be exactly {a} {unit} long (got {n})"),
        (Some(a), Some(b)) => format!("must be {a} to {b} {unit} long (got {n})"),
        (Some(a), None) => format!("must be at least {a} {unit} long (got {n})"),
        (None, Some(b)) => format!("must be at most {b} {unit} long (got {n})"),
        (None, None) => format!("has invalid length {n}"),
    };
    let mut params = Value::map();
    if let Some(a) = min {
        params.insert("min", a);
    }
    if let Some(b) = max {
        params.insert("max", b);
    }
    out.push(Violation {
        path: path.as_str().to_owned(),
        rule: "len",
        message,
        params,
    });
}

/// Values the `range` rule can check, resolving the bound type from the field.
#[diagnostic::on_unimplemented(
    message = "`#[validate(range = ...)]` does not apply to `{Self}`",
    label = "this field is not a number",
    note = "`range` checks integers and floats (or an `Option` of one), with bounds of the same type; use `len` for strings and lists"
)]
pub trait RangeValue<T> {
    /// The value to compare, or `None` to skip (an absent `Option`).
    fn range_value(&self) -> Option<&T>;
}

macro_rules! range_value {
    ($($t:ty),*) => {$(
        impl RangeValue<$t> for $t {
            fn range_value(&self) -> Option<&$t> { Some(self) }
        }
        impl RangeValue<$t> for Option<$t> {
            fn range_value(&self) -> Option<&$t> { self.as_ref() }
        }
    )*};
}
range_value!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64
);

/// Numeric bound conversion for messages and error details.
pub trait RangeBound: PartialOrd + Copy + std::fmt::Display {
    /// As a detail value.
    fn to_value(self) -> Value;
}

macro_rules! range_bound {
    ($($t:ty => $conv:expr),*) => {$(
        impl RangeBound for $t {
            fn to_value(self) -> Value { ($conv)(self) }
        }
    )*};
}
range_bound!(
    i8 => Value::from, i16 => Value::from, i32 => Value::from, i64 => Value::from,
    i128 => |v: i128| i64::try_from(v).map(Value::Int).unwrap_or_else(|_| Value::String(v.to_string())),
    isize => |v: isize| Value::Int(v as i64),
    u8 => Value::from, u16 => Value::from, u32 => Value::from, u64 => Value::from,
    u128 => |v: u128| u64::try_from(v).map(Value::from).unwrap_or_else(|_| Value::String(v.to_string())),
    usize => |v: usize| Value::from(v as u64),
    f32 => Value::from, f64 => Value::from
);

/// The `range` rule.
pub fn range<T: RangeBound, V: RangeValue<T> + ?Sized, R: RangeBounds<T>>(
    value: &V,
    range: R,
    path: &FieldPath,
    out: &mut Violations,
) {
    let Some(v) = value.range_value() else { return };
    // `contains` is false for NaN, which is what we want.
    if range.contains(v) {
        return;
    }
    let mut params = Value::map();
    let lo = match range.start_bound() {
        Bound::Included(a) => {
            params.insert("min", a.to_value());
            Some(format!("at least {a}"))
        }
        Bound::Excluded(a) => {
            params.insert("exclusiveMin", a.to_value());
            Some(format!("greater than {a}"))
        }
        Bound::Unbounded => None,
    };
    let hi = match range.end_bound() {
        Bound::Included(b) => {
            params.insert("max", b.to_value());
            Some(format!("at most {b}"))
        }
        Bound::Excluded(b) => {
            params.insert("exclusiveMax", b.to_value());
            Some(format!("less than {b}"))
        }
        Bound::Unbounded => None,
    };
    let bounds = match (lo, hi) {
        (Some(a), Some(b)) => format!("{a} and {b}"),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => "a number".into(),
    };
    out.push(Violation {
        path: path.as_str().to_owned(),
        rule: "range",
        message: format!("must be {bounds} (got {v})"),
        params,
    });
}

/// The schema constraint of `len = <range>`.
pub fn len_constraint<R: RangeBounds<usize>>(range: R) -> Constraint {
    let (min, max) = len_bounds(&range);
    Constraint::Len { min, max }
}

/// The schema constraint of `range = <range>` on a field of type `V`
/// (`PhantomData::<fn(&V)>` names the field type so the bound type is
/// inferred the same way as for [`range`]).
pub fn range_constraint<T: RangeBound, V: RangeValue<T> + ?Sized, R: RangeBounds<T>>(
    _field: PhantomData<fn(&V)>,
    range: R,
) -> Constraint {
    let bound = |b: Bound<&T>| match b {
        Bound::Included(v) => (Some(v.to_value()), false),
        Bound::Excluded(v) => (Some(v.to_value()), true),
        Bound::Unbounded => (None, false),
    };
    let (min, exclusive_min) = bound(range.start_bound());
    let (max, exclusive_max) = bound(range.end_bound());
    Constraint::Range {
        min,
        exclusive_min,
        max,
        exclusive_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(f: impl FnOnce(&FieldPath, &mut Violations)) -> Vec<String> {
        let mut out = Violations::new();
        f(&FieldPath::root(), &mut out);
        out.iter().map(|v| v.message.clone()).collect()
    }

    #[test]
    fn len_counts_characters_and_skips_none() {
        assert!(check(|p, o| len(&"héllo".to_string(), 1..=5, p, o)).is_empty());
        assert_eq!(
            check(|p, o| len(&String::new(), 1..=80, p, o)),
            vec!["must be 1 to 80 characters long (got 0)"]
        );
        assert_eq!(
            check(|p, o| len(&vec![1, 2, 3], ..3, p, o)),
            vec!["must be at most 2 items long (got 3)"]
        );
        assert!(check(|p, o| len(&None::<String>, 1..=3, p, o)).is_empty());
    }

    #[test]
    fn constraints_for_schemas() {
        assert_eq!(
            len_constraint(1..=80),
            Constraint::Len {
                min: Some(1),
                max: Some(80)
            }
        );
        assert_eq!(
            range_constraint(PhantomData::<fn(&Option<u32>)>, 30..=240),
            Constraint::Range {
                min: Some(Value::Int(30)),
                exclusive_min: false,
                max: Some(Value::Int(240)),
                exclusive_max: false
            }
        );
        assert_eq!(
            range_constraint(PhantomData::<fn(&f64)>, 0.0..1.0),
            Constraint::Range {
                min: Some(Value::Float(0.0)),
                exclusive_min: false,
                max: Some(Value::Float(1.0)),
                exclusive_max: true
            }
        );
    }

    #[test]
    fn range_numbers_and_nan() {
        assert!(check(|p, o| range(&120u32, 1..=240, p, o)).is_empty());
        assert_eq!(
            check(|p, o| range(&0u32, 1..=240, p, o)),
            vec!["must be at least 1 and at most 240 (got 0)"]
        );
        assert_eq!(check(|p, o| range(&f64::NAN, 0.0..=1.0, p, o)).len(), 1);
        assert!(check(|p, o| range(&Some(0.5f32), 0.0..1.0, p, o)).is_empty());
        assert!(check(|p, o| range(&None::<i64>, 0..1, p, o)).is_empty());
    }
}
