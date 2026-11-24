//! Extractor wrappers shared by `orbic` handlers and `serge` route components.

use std::ops::{Deref, DerefMut};

macro_rules! extractor {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name<T>(pub T);

        impl<T> $name<T> {
            /// The wrapped value.
            pub fn into_inner(self) -> T {
                self.0
            }
        }

        impl<T> Deref for $name<T> {
            type Target = T;
            fn deref(&self) -> &T {
                &self.0
            }
        }

        impl<T> DerefMut for $name<T> {
            fn deref_mut(&mut self) -> &mut T {
                &mut self.0
            }
        }

        impl<T> From<T> for $name<T> {
            fn from(value: T) -> Self {
                $name(value)
            }
        }
    };
}

extractor!(
    /// Path parameters as a model: fields fill `{name}` placeholders of the
    /// bound path (`/projects/{id}`).
    Path
);

extractor!(
    /// Query parameters as a model.
    Query
);
