//! Port of `lib/remote/Store.java`.

use std::fmt;

/// Identifies one of the two stores in a remote or disk cache.
// Java: com/google/devtools/build/lib/remote/Store.java:17 Store
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Store {
    /// Action cache.
    Ac,
    /// Content-addressed storage.
    Cas,
}

impl fmt::Display for Store {
    // Java: com/google/devtools/build/lib/remote/Store.java:31 Store.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Store::Ac => "ac",
            Store::Cas => "cas",
        })
    }
}
