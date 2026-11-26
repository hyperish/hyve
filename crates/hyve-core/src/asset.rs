/// A package resource embedded by `hyve::asset!` at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asset {
    path: &'static str,
    bytes: &'static [u8],
}

impl Asset {
    /// Constructs an asset from the path and contents emitted by the macro.
    #[doc(hidden)]
    pub const fn embedded(path: &'static str, bytes: &'static [u8]) -> Self {
        Self { path, bytes }
    }

    /// Logical path in the declaring package.
    pub const fn path(self) -> &'static str {
        self.path
    }

    /// The contents, available on every target without filesystem access.
    pub const fn bytes(self) -> &'static [u8] {
        self.bytes
    }

    /// An embedded asset was checked when its package compiled.
    pub const fn exists(self) -> bool {
        true
    }
}
