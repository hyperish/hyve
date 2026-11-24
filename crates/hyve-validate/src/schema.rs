//! Protocol-neutral type and constraint metadata.
//!
//! Nothing here knows about HTTP or OpenAPI; `orbic-openapi` translates a
//! [`SchemaRegistry`] into an API document, applying the JSON codec's rules
//! (for example 64-bit integers that may travel as strings).

use std::any::type_name;
use std::collections::{BTreeMap, HashMap};

use hyve_core::{Id, Timestamp, Ulid, Value};

/// The shape of a value.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaNode {
    /// `()` / `null`.
    Null,
    /// Boolean.
    Bool,
    /// Integer of the given width.
    Integer {
        /// Bit width.
        bits: u8,
        /// Signedness.
        signed: bool,
    },
    /// Floating point of the given width.
    Float {
        /// Bit width.
        bits: u8,
    },
    /// Text; `format` names well-known textual forms (`ulid`, `date-time`, `char`).
    String {
        /// Format name.
        format: Option<&'static str>,
    },
    /// List of items; `len` fixes the length of arrays.
    List {
        /// Item shape.
        items: Box<SchemaNode>,
        /// Exact length for fixed-size arrays.
        len: Option<u64>,
    },
    /// Map from string keys.
    Map {
        /// Value shape.
        values: Box<SchemaNode>,
    },
    /// May be `null`.
    Nullable(Box<SchemaNode>),
    /// Any value (`Value`).
    Any,
    /// Reference to a named definition in the registry.
    Ref(String),
}

/// A rule attached to a field.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// Inclusive length bounds.
    Len {
        /// Minimum.
        min: Option<u64>,
        /// Maximum.
        max: Option<u64>,
    },
    /// Numeric bounds.
    Range {
        /// Lower bound.
        min: Option<Value>,
        /// Whether the lower bound is excluded.
        exclusive_min: bool,
        /// Upper bound.
        max: Option<Value>,
        /// Whether the upper bound is excluded.
        exclusive_max: bool,
    },
}

/// One field of an object definition.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSchema {
    /// Name on the wire (`camelCase`).
    pub json_name: &'static str,
    /// Shape.
    pub schema: SchemaNode,
    /// Whether the field must be present (`false` for `Option` fields).
    pub required: bool,
    /// Rules from `#[validate]`.
    pub constraints: Vec<Constraint>,
}

/// A named definition.
#[derive(Debug, Clone, PartialEq)]
pub enum Definition {
    /// An object with fields.
    Object(Vec<FieldSchema>),
    /// A string enum with these values.
    Enum(Vec<&'static str>),
}

/// Collects named definitions while schemas are built; recursion through
/// references terminates because a name is reserved before its body is built.
#[derive(Debug, Default)]
pub struct SchemaRegistry {
    names: HashMap<&'static str, String>,
    definitions: BTreeMap<String, Option<Definition>>,
}

impl SchemaRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `T` under `short_name` (made unique if another type already
    /// uses it) and returns a reference to it. `build` runs only the first time.
    pub fn define<T: ?Sized + 'static>(
        &mut self,
        short_name: &'static str,
        build: impl FnOnce(&mut SchemaRegistry) -> Definition,
    ) -> SchemaNode {
        let key = type_name::<T>();
        if let Some(name) = self.names.get(key) {
            return SchemaNode::Ref(name.clone());
        }
        let mut name = short_name.to_owned();
        let mut n = 2;
        while self.definitions.contains_key(&name) {
            name = format!("{short_name}{n}");
            n += 1;
        }
        self.names.insert(key, name.clone());
        self.definitions.insert(name.clone(), None);
        let def = build(self);
        self.definitions.insert(name.clone(), Some(def));
        SchemaNode::Ref(name)
    }

    /// The schema of `T`, registering whatever it references.
    pub fn schema_of<T: Schema + ?Sized>(&mut self) -> SchemaNode {
        T::schema(self)
    }

    /// Finished definitions by name.
    pub fn definitions(&self) -> impl Iterator<Item = (&str, &Definition)> {
        self.definitions
            .iter()
            .filter_map(|(k, v)| v.as_ref().map(|d| (k.as_str(), d)))
    }

    /// One definition by name.
    pub fn get(&self, name: &str) -> Option<&Definition> {
        self.definitions.get(name).and_then(Option::as_ref)
    }
}

/// Types that can describe their shape.
pub trait Schema {
    /// The shape of `Self`; named types register themselves in `registry`.
    fn schema(registry: &mut SchemaRegistry) -> SchemaNode;

    /// Whether a field of this type may be omitted (only `Option`).
    fn optional() -> bool {
        false
    }
}

macro_rules! leaf {
    ($($t:ty => $node:expr),* $(,)?) => {$(
        impl Schema for $t {
            fn schema(_: &mut SchemaRegistry) -> SchemaNode { $node }
        }
    )*};
}

leaf! {
    () => SchemaNode::Null,
    bool => SchemaNode::Bool,
    i8 => SchemaNode::Integer { bits: 8, signed: true },
    i16 => SchemaNode::Integer { bits: 16, signed: true },
    i32 => SchemaNode::Integer { bits: 32, signed: true },
    i64 => SchemaNode::Integer { bits: 64, signed: true },
    i128 => SchemaNode::Integer { bits: 128, signed: true },
    u8 => SchemaNode::Integer { bits: 8, signed: false },
    u16 => SchemaNode::Integer { bits: 16, signed: false },
    u32 => SchemaNode::Integer { bits: 32, signed: false },
    u64 => SchemaNode::Integer { bits: 64, signed: false },
    u128 => SchemaNode::Integer { bits: 128, signed: false },
    f32 => SchemaNode::Float { bits: 32 },
    f64 => SchemaNode::Float { bits: 64 },
    char => SchemaNode::String { format: Some("char") },
    String => SchemaNode::String { format: None },
    Ulid => SchemaNode::String { format: Some("ulid") },
    Timestamp => SchemaNode::String { format: Some("date-time") },
    Value => SchemaNode::Any,
}

impl<T: ?Sized> Schema for Id<T> {
    fn schema(_: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::String {
            format: Some("ulid"),
        }
    }
}

impl<T: Schema> Schema for Option<T> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::Nullable(Box::new(T::schema(r)))
    }
    fn optional() -> bool {
        true
    }
}

impl<T: Schema> Schema for Box<T> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        T::schema(r)
    }
}

impl<T: Schema> Schema for Vec<T> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::List {
            items: Box::new(T::schema(r)),
            len: None,
        }
    }
}

impl<T: Schema, const N: usize> Schema for [T; N] {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::List {
            items: Box::new(T::schema(r)),
            len: Some(N as u64),
        }
    }
}

impl<V: Schema> Schema for BTreeMap<String, V> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::Map {
            values: Box::new(V::schema(r)),
        }
    }
}

impl<V: Schema, S> Schema for HashMap<String, V, S> {
    fn schema(r: &mut SchemaRegistry) -> SchemaNode {
        SchemaNode::Map {
            values: Box::new(V::schema(r)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recursive type, as `#[model]` would describe it.
    struct Node {
        _children: Vec<Node>,
    }

    impl Schema for Node {
        fn schema(r: &mut SchemaRegistry) -> SchemaNode {
            r.define::<Node>("Node", |r| {
                Definition::Object(vec![FieldSchema {
                    json_name: "children",
                    schema: <Vec<Node>>::schema(r),
                    required: true,
                    constraints: vec![Constraint::Len {
                        min: None,
                        max: Some(8),
                    }],
                }])
            })
        }
    }

    #[test]
    fn recursion_terminates_through_refs() {
        let mut r = SchemaRegistry::new();
        assert_eq!(r.schema_of::<Node>(), SchemaNode::Ref("Node".into()));
        let Some(Definition::Object(fields)) = r.get("Node") else {
            panic!()
        };
        assert_eq!(
            fields[0].schema,
            SchemaNode::List {
                items: Box::new(SchemaNode::Ref("Node".into())),
                len: None
            }
        );
        assert_eq!(r.definitions().count(), 1);
    }

    #[test]
    fn colliding_short_names_get_suffixes() {
        mod a {
            pub struct Item;
        }
        mod b {
            pub struct Item;
        }
        let mut r = SchemaRegistry::new();
        let x = r.define::<a::Item>("Item", |_| Definition::Enum(vec!["x"]));
        let y = r.define::<b::Item>("Item", |_| Definition::Enum(vec!["y"]));
        assert_eq!(
            (x, y),
            (
                SchemaNode::Ref("Item".into()),
                SchemaNode::Ref("Item2".into())
            )
        );
        assert!(<Option<u8>>::optional());
    }
}
