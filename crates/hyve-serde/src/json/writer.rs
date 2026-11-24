//! Streaming JSON writer.

use std::fmt::Write as _;

/// Largest integer magnitude a JSON number carries exactly (2^53).
pub(crate) const EXACT_INT: u128 = 1 << 53;

/// Writes one JSON value into a `String`, handling commas and escaping.
#[derive(Debug, Default)]
pub struct JsonWriter {
    out: String,
    /// One entry per open container: whether it already has an item.
    has_items: Vec<bool>,
    after_key: bool,
}

impl JsonWriter {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The written text.
    pub fn as_str(&self) -> &str {
        &self.out
    }

    /// Consumes the writer.
    pub fn into_string(self) -> String {
        self.out
    }

    fn before_value(&mut self) {
        if self.after_key {
            self.after_key = false;
            return;
        }
        if let Some(has) = self.has_items.last_mut() {
            if *has {
                self.out.push(',');
            }
            *has = true;
        }
    }

    /// `{`
    pub fn begin_object(&mut self) {
        self.before_value();
        self.out.push('{');
        self.has_items.push(false);
    }

    /// Writes an object key; the next call must write its value.
    pub fn key(&mut self, key: &str) {
        if let Some(has) = self.has_items.last_mut() {
            if *has {
                self.out.push(',');
            }
            *has = true;
        }
        write_escaped(&mut self.out, key);
        self.out.push(':');
        self.after_key = true;
    }

    /// `}`
    pub fn end_object(&mut self) {
        self.has_items.pop();
        self.out.push('}');
    }

    /// `[`
    pub fn begin_array(&mut self) {
        self.before_value();
        self.out.push('[');
        self.has_items.push(false);
    }

    /// `]`
    pub fn end_array(&mut self) {
        self.has_items.pop();
        self.out.push(']');
    }

    /// `null`
    pub fn null(&mut self) {
        self.before_value();
        self.out.push_str("null");
    }

    /// `true` / `false`
    pub fn bool(&mut self, v: bool) {
        self.before_value();
        self.out.push_str(if v { "true" } else { "false" });
    }

    /// A string value.
    pub fn string(&mut self, v: &str) {
        self.before_value();
        write_escaped(&mut self.out, v);
    }

    /// A signed integer (decimal string when |v| > 2^53).
    pub fn int(&mut self, v: i128) {
        self.before_value();
        if v.unsigned_abs() <= EXACT_INT {
            let _ = write!(self.out, "{v}");
        } else {
            let _ = write!(self.out, "\"{v}\"");
        }
    }

    /// An unsigned integer (decimal string when v > 2^53).
    pub fn uint(&mut self, v: u128) {
        self.before_value();
        if v <= EXACT_INT {
            let _ = write!(self.out, "{v}");
        } else {
            let _ = write!(self.out, "\"{v}\"");
        }
    }

    /// A float; non-finite values become `"NaN"`, `"Infinity"`, `"-Infinity"`.
    pub fn float(&mut self, v: f64) {
        self.before_value();
        if v.is_nan() {
            self.out.push_str("\"NaN\"");
        } else if v.is_infinite() {
            self.out.push_str(if v > 0.0 {
                "\"Infinity\""
            } else {
                "\"-Infinity\""
            });
        } else {
            let a = v.abs();
            // Shortest round-trip digits; exponent form only for extreme magnitudes.
            if a == 0.0 || (1e-5..1e16).contains(&a) {
                let _ = write!(self.out, "{v}");
            } else {
                let _ = write!(self.out, "{v:e}");
            }
        }
    }

    /// A float that is known to be `f32` (printed with `f32` precision).
    pub fn float32(&mut self, v: f32) {
        if !v.is_finite() {
            return self.float(v as f64);
        }
        self.before_value();
        let a = v.abs();
        if a == 0.0 || (1e-5..1e16).contains(&a) {
            let _ = write!(self.out, "{v}");
        } else {
            let _ = write!(self.out, "{v:e}");
        }
    }
}

fn write_escaped(out: &mut String, s: &str) {
    out.push('"');
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        let esc: Option<&str> = match b {
            b'"' => Some("\\\""),
            b'\\' => Some("\\\\"),
            b'\n' => Some("\\n"),
            b'\r' => Some("\\r"),
            b'\t' => Some("\\t"),
            0x08 => Some("\\b"),
            0x0c => Some("\\f"),
            0x00..=0x1f | 0x7f => None,
            _ => continue,
        };
        out.push_str(&s[start..i]);
        match esc {
            Some(e) => out.push_str(e),
            None => {
                let _ = write!(out, "\\u{b:04x}");
            }
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commas_and_nesting() {
        let mut w = JsonWriter::new();
        w.begin_object();
        w.key("a");
        w.int(1);
        w.key("b");
        w.begin_array();
        w.bool(true);
        w.null();
        w.begin_object();
        w.end_object();
        w.end_array();
        w.key("c");
        w.string("x\"y\n\u{1}");
        w.end_object();
        assert_eq!(
            w.as_str(),
            r#"{"a":1,"b":[true,null,{}],"c":"x\"y\n\u0001"}"#
        );
    }

    #[test]
    fn numbers() {
        let mut w = JsonWriter::new();
        w.begin_array();
        w.int(9_007_199_254_740_992);
        w.int(9_007_199_254_740_993);
        w.uint(u64::MAX as u128);
        w.float(0.1);
        w.float(3.0);
        w.float(1e300);
        w.float(-0.0);
        w.float(f64::NAN);
        w.float(f64::NEG_INFINITY);
        w.float32(0.1);
        w.end_array();
        assert_eq!(
            w.as_str(),
            r#"[9007199254740992,"9007199254740993","18446744073709551615",0.1,3,1e300,-0,"NaN","-Infinity",0.1]"#
        );
    }
}
