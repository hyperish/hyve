//! `Json` for standard and `hyve-core` types.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::BuildHasher;

use hyve_core::{Id, Timestamp, Ulid, Value};

use super::{Json, JsonReader, JsonWriter, Peek};
use crate::DecodeError;

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl Json for $t {
            fn encode_json(&self, w: &mut JsonWriter) { w.int(*self as i128) }
            fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
                let v = r.read_int()?;
                <$t>::try_from(v).map_err(|_| r.error(format!("{v} does not fit `{}`", stringify!($t))))
            }
        }
    )*};
}
signed!(i8, i16, i32, i64, i128, isize);

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl Json for $t {
            fn encode_json(&self, w: &mut JsonWriter) { w.uint(*self as u128) }
            fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
                let v = r.read_uint()?;
                <$t>::try_from(v).map_err(|_| r.error(format!("{v} does not fit `{}`", stringify!($t))))
            }
        }
    )*};
}
unsigned!(u8, u16, u32, u64, u128, usize);

impl Json for f64 {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.float(*self)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.read_float()
    }
}

impl Json for f32 {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.float32(*self)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        let v = r.read_float()?;
        if v.is_finite() && v.abs() > f32::MAX as f64 {
            return Err(r.error(format!("{v} does not fit `f32`")));
        }
        Ok(v as f32)
    }
}

impl Json for bool {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.bool(*self)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.read_bool()
    }
}

impl Json for () {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.null()
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.read_null()
    }
}

impl Json for String {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.string(self)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.read_string().map(|s| s.into_owned())
    }
}

impl Json for char {
    fn encode_json(&self, w: &mut JsonWriter) {
        let mut buf = [0u8; 4];
        w.string(self.encode_utf8(&mut buf))
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        let s = r.read_string()?;
        let mut it = s.chars();
        match (it.next(), it.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(r.error("expected a single character")),
        }
    }
}

impl<T: Json> Json for Option<T> {
    fn encode_json(&self, w: &mut JsonWriter) {
        match self {
            Some(v) => v.encode_json(w),
            None => w.null(),
        }
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        if r.peek()? == Peek::Null {
            r.read_null()?;
            Ok(None)
        } else {
            T::decode_json(r).map(Some)
        }
    }
}

impl<T: Json> Json for Box<T> {
    fn encode_json(&self, w: &mut JsonWriter) {
        (**self).encode_json(w)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        T::decode_json(r).map(Box::new)
    }
}

impl<T: Json> Json for Vec<T> {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_array();
        for v in self {
            v.encode_json(w);
        }
        w.end_array();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_array()?;
        let mut out = Vec::new();
        while r.next_element()? {
            out.push(T::decode_json(r)?);
        }
        Ok(out)
    }
}

impl<T: Json, const N: usize> Json for [T; N] {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_array();
        for v in self {
            v.encode_json(w);
        }
        w.end_array();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        let v = Vec::<T>::decode_json(r)?;
        let len = v.len();
        v.try_into()
            .map_err(|_| r.error(format!("expected {N} elements, found {len}")))
    }
}

impl<V: Json> Json for BTreeMap<String, V> {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        for (k, v) in self {
            w.key(k);
            v.encode_json(w);
        }
        w.end_object();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut out = BTreeMap::new();
        while let Some(k) = r.next_key()? {
            let v = V::decode_json(r)?;
            if out.insert(k.clone().into_owned(), v).is_some() {
                return Err(r.duplicate_field(&k));
            }
        }
        Ok(out)
    }
}

impl<V: Json, S: BuildHasher + Default> Json for HashMap<String, V, S> {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.begin_object();
        for (k, v) in self {
            w.key(k);
            v.encode_json(w);
        }
        w.end_object();
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        r.begin_object()?;
        let mut out = HashMap::with_hasher(S::default());
        while let Some(k) = r.next_key()? {
            let v = V::decode_json(r)?;
            if out.insert(k.clone().into_owned(), v).is_some() {
                return Err(r.duplicate_field(&k));
            }
        }
        Ok(out)
    }
}

impl Json for Ulid {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.string(&self.to_string())
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        let s = r.read_string()?;
        s.parse().map_err(|e| r.error(format!("{e}")))
    }
}

impl<T: ?Sized> Json for Id<T> {
    fn encode_json(&self, w: &mut JsonWriter) {
        self.ulid().encode_json(w)
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        Ulid::decode_json(r).map(Id::from_ulid)
    }
}

impl Json for Timestamp {
    fn encode_json(&self, w: &mut JsonWriter) {
        w.string(&self.to_string())
    }
    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        let s = r.read_string()?;
        s.parse().map_err(|e| r.error(format!("{e}")))
    }
}

impl Json for Value {
    fn encode_json(&self, w: &mut JsonWriter) {
        match self {
            Value::Null => w.null(),
            Value::Bool(b) => w.bool(*b),
            Value::Int(i) => w.int(*i as i128),
            Value::UInt(u) => w.uint(*u as u128),
            Value::Float(f) => w.float(*f),
            Value::String(s) => w.string(s),
            Value::List(items) => {
                w.begin_array();
                for v in items {
                    v.encode_json(w);
                }
                w.end_array();
            }
            Value::Map(entries) => {
                w.begin_object();
                for (k, v) in entries {
                    w.key(k);
                    v.encode_json(w);
                }
                w.end_object();
            }
        }
    }

    fn decode_json(r: &mut JsonReader<'_>) -> Result<Self, DecodeError> {
        Ok(match r.peek()? {
            Peek::Null => {
                r.read_null()?;
                Value::Null
            }
            Peek::Bool => Value::Bool(r.read_bool()?),
            Peek::Number => {
                let t = r.read_number()?;
                if let Ok(i) = t.parse::<i64>() {
                    Value::Int(i)
                } else if let Ok(u) = t.parse::<u64>() {
                    Value::UInt(u)
                } else {
                    Value::Float(t.parse().map_err(|_| r.error("invalid number"))?)
                }
            }
            Peek::String => Value::String(r.read_string()?.into_owned()),
            Peek::Array => {
                r.begin_array()?;
                let mut items = Vec::new();
                while r.next_element()? {
                    items.push(Value::decode_json(r)?);
                }
                Value::List(items)
            }
            Peek::Object => {
                r.begin_object()?;
                let mut entries: Vec<(String, Value)> = Vec::new();
                let mut seen: HashSet<String> = HashSet::new();
                while let Some(k) = r.next_key()? {
                    if !seen.insert(k.clone().into_owned()) {
                        return Err(r.duplicate_field(&k));
                    }
                    let v = Value::decode_json(r)?;
                    entries.push((k.into_owned(), v));
                }
                Value::Map(entries)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{JsonLimits, from_json, from_json_with, to_json};
    use super::*;

    fn rt<T: Json + PartialEq + std::fmt::Debug>(v: T) {
        let s = to_json(&v);
        assert_eq!(from_json::<T>(&s).unwrap(), v, "{s}");
    }

    #[test]
    fn roundtrips() {
        rt(true);
        rt(i64::MIN);
        rt(u64::MAX);
        rt(i128::MAX);
        rt(-0.5f64);
        rt(1.0e-300f64);
        rt(0.1f32);
        rt(String::from("héllo \"\u{1F600}\"\n"));
        rt('ß');
        rt(Some(3u8));
        rt(None::<u8>);
        rt(vec![vec![1u16, 2], vec![]]);
        rt([1i32, 2, 3]);
        rt(BTreeMap::from([
            ("a".to_string(), 1u8),
            ("b".to_string(), 2),
        ]));
        rt(Ulid::new());
        rt(Timestamp::now());
        // Schemaless `Value`: an integer beyond 2^53 is written as a string and
        // reads back as `Value::String` (typed fields read it back exactly).
        let mut big = Value::Null;
        big.insert("big", u64::MAX);
        assert_eq!(
            from_json::<Value>(&to_json(&big)).unwrap().get("big"),
            Some(&Value::String(u64::MAX.to_string()))
        );
        let mut v = Value::Null;
        v.insert("big", 1u64 << 53);
        v.insert("list", vec![1i64, -2]);
        v.insert("f", 1.5);
        rt(v);
    }

    #[test]
    fn non_finite_floats_travel_as_strings() {
        assert_eq!(to_json(&f64::INFINITY), "\"Infinity\"");
        assert!(from_json::<f64>("\"NaN\"").unwrap().is_nan());
        assert!(from_json::<f64>("\"nan\"").is_err());
    }

    #[test]
    fn integers_accept_number_or_string() {
        assert_eq!(
            from_json::<u64>("\"18446744073709551615\"").unwrap(),
            u64::MAX
        );
        assert_eq!(from_json::<i64>("42").unwrap(), 42);
        for bad in ["1.0", "1e3", "\"01\"", "\"+1\"", "\"\"", "-0.5"] {
            assert!(from_json::<i64>(bad).is_err(), "{bad}");
        }
        assert!(from_json::<u8>("256").is_err());
        assert!(from_json::<u32>("-1").is_err());
    }

    #[test]
    fn strict_syntax() {
        for bad in [
            "",
            "[1,]",
            "[,1]",
            "{\"a\":1,}",
            "{\"a\" 1}",
            "{'a':1}",
            "01",
            "1.",
            ".5",
            "+1",
            "[1] x",
            "\"\\x\"",
            "\"\\ud800\"",
            "\"a\u{1}\"",
            "nul",
            "{\"a\":1 \"b\":2}",
            "[1 2]",
            "\"open",
        ] {
            assert!(from_json::<Value>(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            from_json::<String>("\"\\ud83d\\ude00\"").unwrap(),
            "\u{1F600}"
        );
        assert_eq!(from_json::<Vec<u8>>(" [ 1 , 2 ] ").unwrap(), vec![1, 2]);
    }

    #[test]
    fn duplicate_keys_rejected() {
        assert!(from_json::<Value>("{\"a\":1,\"a\":2}").is_err());
        assert!(from_json::<BTreeMap<String, u8>>("{\"a\":1,\"a\":2}").is_err());
    }

    #[test]
    fn limits_apply() {
        let deep = "[".repeat(200) + &"]".repeat(200);
        let e = from_json::<Value>(&deep).unwrap_err();
        assert!(e.message().contains("nesting"), "{e}");
        let small = JsonLimits {
            max_depth: 4,
            max_len: 10,
        };
        assert!(from_json_with::<Value>("[1,2,3,4,5,6]", small).is_err());
    }

    #[test]
    fn error_paths() {
        let e = from_json::<Vec<BTreeMap<String, u8>>>("[{\"a\":1},{\"b\":\"x\"}]").unwrap_err();
        assert_eq!(e.path(), "$[1].b");
    }
}
