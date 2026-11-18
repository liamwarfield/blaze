//! `AspectResolver.Mode` from `com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/query2/query/aspectresolvers/AspectResolver.java:32 AspectResolver.Mode
option_enum! {
    /// How to resolve aspect dependencies in 'blaze query'.
    // PORT: the per-constant createResolver bodies belong to query, not ported here.
    pub enum AspectResolverMode { OFF, CONSERVATIVE, PRECISE }
}
