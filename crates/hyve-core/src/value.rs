//! Protocol-neutral structured data.

/// Structured, protocol-neutral data (the shape of JSON without committing to JSON).
///
/// Used for [`HyveError`](crate::HyveError) details so both the binary and the
/// JSON rail can carry them without loss. Maps keep insertion order.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// Absence of a value.
    #[default]
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Int(i64),
    /// Unsigned integer that does not fit `i64`.
    UInt(u64),
    /// Floating-point number (may be non-finite).
    Float(f64),
    /// UTF-8 text.
    String(String),
    /// Ordered list.
    List(Vec<Value>),
    /// Ordered map with string keys.
    Map(Vec<(String, Value)>),
}

impl Value {
    /// Returns an empty map.
    pub fn map() -> Self {
        Value::Map(Vec::new())
    }

    /// Inserts or replaces `key` when `self` is a map; turns `Null` into a map first.
    ///
    /// # Panics
    ///
    /// Panics if `self` is neither `Null` nor a map.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        if matches!(self, Value::Null) {
            *self = Value::map();
        }
        let Value::Map(entries) = self else {
            panic!("Value::insert called on a non-map value");
        };
        let key = key.into();
        let value = value.into();
        match entries.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => entries.push((key, value)),
        }
    }

    /// Looks up `key` when `self` is a map.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Returns `true` for `Null` and for an empty map.
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Null => true,
            Value::Map(m) => m.is_empty(),
            _ => false,
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

macro_rules! from_signed {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Self { Value::Int(v as i64) }
        }
    )*};
}
from_signed!(i8, i16, i32, i64);

macro_rules! from_unsigned {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(v: $t) -> Self { Value::Int(v as i64) }
        }
    )*};
}
from_unsigned!(u8, u16, u32);

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        match i64::try_from(v) {
            Ok(i) => Value::Int(i),
            Err(_) => Value::UInt(v),
        }
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Float(v as f64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn insert_replaces_and_keeps_order() {
        let mut v = Value::Null;
        v.insert("a", 1);
        v.insert("b", "x");
        v.insert("a", 2);
        assert_eq!(
            v,
            Value::Map(vec![("a".into(), Value::Int(2)), ("b".into(), "x".into())])
        );
        assert_eq!(v.get("b"), Some(&Value::String("x".into())));
    }

    #[test]
    fn large_unsigned_stays_unsigned() {
        assert_eq!(Value::from(u64::MAX), Value::UInt(u64::MAX));
        assert_eq!(Value::from(5u64), Value::Int(5));
    }
}
