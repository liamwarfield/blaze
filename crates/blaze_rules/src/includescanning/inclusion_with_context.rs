//! `InclusionWithContext` and `IncludeScannerHelper`.

use std::fmt;
use std::hash::{Hash, Hasher};

use blaze_actions::Artifact;
use blaze_util::check_argument;
use blaze_vfs::PathFragment;

use super::include_parser::{Inclusion, Kind};

/// An [`Inclusion`] together with the context where on the include path the inclusion was
/// found, and whether the containing file was included using angle brackets or quotes.
// Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:25 InclusionWithContext
#[derive(Clone, Debug)]
pub struct InclusionWithContext {
    inclusion: Inclusion,
    context_path_pos: i32,
    context_kind: Option<Kind>,
}

impl InclusionWithContext {
    /// Attaches context to an inclusion.
    ///
    /// `context_path_pos` is the position on the include path on which the containing file was
    /// found; `context_kind` is how the containing file was included (used only for include_next
    /// inclusions; must not be a `NEXT_*` kind).
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:40 InclusionWithContext.<init>
    pub fn new(
        inclusion: Inclusion,
        context_path_pos: i32,
        context_kind: Option<Kind>,
    ) -> InclusionWithContext {
        check_argument!(context_kind.is_none_or(|k| !k.is_next()), "{}", inclusion);
        // The context kind is only stored for #include_next inclusions.
        let context_kind = if inclusion.kind.is_next() {
            context_kind
        } else {
            Some(inclusion.kind)
        };
        InclusionWithContext {
            inclusion,
            context_path_pos,
            context_kind,
        }
    }

    /// Creates a simple `QUOTE` or `ANGLE` inclusion with empty context.
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:40 InclusionWithContext.<init>
    pub fn simple(name: &str, kind: Kind) -> InclusionWithContext {
        InclusionWithContext::new(Inclusion::create(name, kind), -1, None)
    }

    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:65 InclusionWithContext.getInclusion
    pub fn get_inclusion(&self) -> &Inclusion {
        &self.inclusion
    }

    /// The position on the include path on which the containing file was found: -1 for a
    /// top-level or not a `#include_next` inclusion, 0 for a `#include_next` inclusion and a
    /// locally found header, >0 for a `#include_next` inclusion found on the include path.
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:78 InclusionWithContext.getContextPathPos
    pub fn get_context_path_pos(&self) -> i32 {
        self.context_path_pos
    }

    /// On which include path to continue searching.
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:88 InclusionWithContext.getContextKind
    pub fn get_context_kind(&self) -> Option<Kind> {
        self.context_kind
    }
}

impl fmt::Display for InclusionWithContext {
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:93 InclusionWithContext.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inclusion.kind.is_next() {
            let kind = self
                .context_kind
                .map_or_else(|| "null".to_owned(), |k| k.to_string());
            write!(f, "{}({}:{})", self.inclusion, kind, self.context_path_pos)
        } else {
            write!(f, "{}", self.inclusion)
        }
    }
}

impl PartialEq for InclusionWithContext {
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:100 InclusionWithContext.equals
    fn eq(&self, that: &Self) -> bool {
        self.inclusion == that.inclusion
            && (!self.inclusion.kind.is_next() || self.context_path_pos == that.context_path_pos)
            && self.context_kind == that.context_kind
    }
}

impl Eq for InclusionWithContext {}

impl Hash for InclusionWithContext {
    // Java: com/google/devtools/build/lib/includescanning/InclusionWithContext.java:113 InclusionWithContext.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inclusion.hash(state);
        if self.inclusion.kind.is_next() {
            self.context_path_pos.hash(state);
        }
        self.context_kind.hash(state);
    }
}

/// Helper class that deals with the processing of a given file's inclusions.
// Java: com/google/devtools/build/lib/includescanning/IncludeScannerHelper.java:26 IncludeScannerHelper
pub struct IncludeScannerHelper<'a> {
    include_paths: &'a [PathFragment],
    quote_include_paths: &'a [PathFragment],
    source: &'a Artifact,
    quote_context_path_pos: Option<i32>,
    angle_context_path_pos: Option<i32>,
}

impl<'a> IncludeScannerHelper<'a> {
    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerHelper.java:36 IncludeScannerHelper.<init>
    pub fn new(
        include_paths: &'a [PathFragment],
        quote_include_paths: &'a [PathFragment],
        source: &'a Artifact,
    ) -> IncludeScannerHelper<'a> {
        IncludeScannerHelper {
            include_paths,
            quote_include_paths,
            source,
            quote_context_path_pos: None,
            angle_context_path_pos: None,
        }
    }

    /// Finds `source` in the given `include_paths` as if it were #included as `include`.
    ///
    /// Returns the index in `include_paths` a #include_next directive should start searching
    /// from, or -1 if the source file was not found.
    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerHelper.java:50 IncludeScannerHelper.findContextPathPos
    fn find_context_path_pos(
        include: &PathFragment,
        source: &Artifact,
        include_paths: &[PathFragment],
    ) -> i32 {
        for (i, exec_path) in include_paths.iter().enumerate() {
            if exec_path.get_relative(include) == *source.get_exec_path() {
                return i as i32 + 1;
            }
        }
        -1
    }

    /// Most inclusions become inclusions with context as-is -- the context of their including
    /// file is their context. However, in the case of a top-level #include_next inclusion, a
    /// more involved procedure is needed to find the proper context.
    // Java: com/google/devtools/build/lib/includescanning/IncludeScannerHelper.java:66 IncludeScannerHelper.createInclusionWithContext
    pub fn create_inclusion_with_context(
        &mut self,
        inclusion: &Inclusion,
        mut context_path_pos: i32,
        mut context_kind: Option<Kind>,
    ) -> InclusionWithContext {
        let mut inclusion = inclusion.clone();
        if inclusion.kind.is_next() && context_path_pos == -1 {
            // Special handling for the case when a #include_next directive is found without
            // context, e. g., in the main source file.

            // First, try to find the source of the inclusion via a header search.
            if inclusion.kind == Kind::NextQuote {
                if self.quote_context_path_pos.is_none() {
                    self.quote_context_path_pos = Some(Self::find_context_path_pos(
                        &inclusion.path_fragment,
                        self.source,
                        self.quote_include_paths,
                    ));
                }
                context_path_pos = self.quote_context_path_pos.unwrap();
            } else {
                if self.angle_context_path_pos.is_none() {
                    self.angle_context_path_pos = Some(Self::find_context_path_pos(
                        &inclusion.path_fragment,
                        self.source,
                        self.include_paths,
                    ));
                }
                context_path_pos = self.angle_context_path_pos.unwrap();
            }
            let kind = if inclusion.kind == Kind::NextQuote {
                Kind::Quote
            } else {
                Kind::Angle
            };
            if context_path_pos != -1 {
                // If the source was found via the header search, assume that is a valid context
                // to start the search from. Assume the context kind is the same as the kind of
                // the include_next directive.
                context_kind = Some(kind);
            } else {
                // If the source was not found, #include_next behaves exactly like #include.
                inclusion = Inclusion::create_from_fragment(inclusion.path_fragment.clone(), kind);
            }
        }
        InclusionWithContext::new(inclusion, context_path_pos, context_kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_kind_is_the_inclusion_kind_unless_next() {
        let i = InclusionWithContext::new(Inclusion::create("a.h", Kind::Angle), 3, None);
        assert_eq!(i.get_context_kind(), Some(Kind::Angle));
        let n = InclusionWithContext::new(
            Inclusion::create("a.h", Kind::NextAngle),
            3,
            Some(Kind::Quote),
        );
        assert_eq!(n.get_context_kind(), Some(Kind::Quote));
        assert_eq!(n.to_string(), "NEXT_ANGLE:a.h(QUOTE:3)");
    }

    #[test]
    fn equality_ignores_position_unless_next() {
        let a = InclusionWithContext::new(Inclusion::create("a.h", Kind::Quote), 1, None);
        let b = InclusionWithContext::new(Inclusion::create("a.h", Kind::Quote), 2, None);
        assert_eq!(a, b);
        let c = InclusionWithContext::new(
            Inclusion::create("a.h", Kind::NextQuote),
            1,
            Some(Kind::Quote),
        );
        let d = InclusionWithContext::new(
            Inclusion::create("a.h", Kind::NextQuote),
            2,
            Some(Kind::Quote),
        );
        assert_ne!(c, d);
    }
}
