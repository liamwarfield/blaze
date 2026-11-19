//! Port of `Version`, `IntVersion` and `NodeVersion`.

use std::fmt;

/// A Version defines a value in a version tree used in persistent data structures. See
/// http://en.wikipedia.org/wiki/Persistent_data_structure.
///
/// Extends [`NodeVersion`] so that a single instance can be reused to save memory in the common
/// case that `lastEvaluated` and `lastChanged` are the same version.
///
/// PORT: Java's `Version` is an interface with three implementations at the pin
/// (`MinimalVersion`, `ConstantVersion`, `IntVersion`) and no other implementor in Bazel; a
/// closed hierarchy is a Rust enum. `MinimalVersion` and `ConstantVersion` are singletons, so
/// Java's identity equality is variant equality here.
// Java: com/google/devtools/build/skyframe/Version.java:23 Version
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Version {
    /// A version [`Version::lower_than`] all other versions, other than itself.
    // Java: com/google/devtools/build/skyframe/Version.java:81 Version.MinimalVersion
    Minimal,
    /// A version that is not comparable to any version other than itself.
    // Java: com/google/devtools/build/skyframe/Version.java:93 Version.ConstantVersion
    Constant,
    // Java: com/google/devtools/build/skyframe/IntVersion.java:20 IntVersion
    Int(IntVersion),
}

impl Version {
    /// Defines a partial order relation on versions. Returns true if this object is at most
    /// `other` in that partial order. If x.equals(y), then x.atMost(y).
    ///
    /// If x.atMost(y) returns false, then there are two possibilities: y < x in the partial
    /// order, so y.atMost(x) returns true and !x.equals(y), or x and y are incomparable in this
    /// partial order. This may be because x and y are instances of different Version
    /// implementations (although it is legal for different Version implementations to be
    /// comparable as well). See http://en.wikipedia.org/wiki/Partially_ordered_set.
    // Java: com/google/devtools/build/skyframe/Version.java:34 Version.atMost
    pub fn at_most(&self, other: &Version) -> bool {
        match self {
            // Java: com/google/devtools/build/skyframe/Version.java:87 MinimalVersion.atMost
            Version::Minimal => true,
            // Java: com/google/devtools/build/skyframe/Version.java:99 ConstantVersion.atMost
            Version::Constant => matches!(other, Version::Constant),
            Version::Int(v) => v.at_most(other),
        }
    }

    /// Returns whether `this < other` in the partial order of versions, similarly to
    /// [`Version::at_most`].
    ///
    /// Returns true iff the 2 versions are comparable in the partial order and `this` is strictly
    /// lower than `other`. False result means that either the elements are comparable and
    /// `this >= other` or the versions are not comparable in the partial order.
    // Java: com/google/devtools/build/skyframe/Version.java:44 Version.lowerThan
    pub fn lower_than(&self, other: &Version) -> bool {
        self.at_most(other) && self != other
    }

    // Java: com/google/devtools/build/skyframe/Version.java:49 Version.lastEvaluated
    pub fn last_evaluated(&self) -> &Version {
        self
    }

    // Java: com/google/devtools/build/skyframe/Version.java:54 Version.lastChanged
    pub fn last_changed(&self) -> &Version {
        self
    }

    /// Returns a version indicating that a `NodeEntry` has never been built.
    ///
    /// The minimal version is never used as the graph version for Skyframe evaluations. It is
    /// only used to indicatie that a node has never been built, since it is always
    /// [`Version::lower_than`] the graph version.
    // Java: com/google/devtools/build/skyframe/Version.java:65 Version.minimal
    pub fn minimal() -> Version {
        Version::Minimal
    }

    /// Returns a version indicating that a `NodeEntry` has been built without incrementality.
    ///
    /// The constant version is used as the graph version for all non-incremental Skyframe
    /// evaluations. Without incrementality, it makes sense to use a version with no defined
    /// "previous" or "next" version.
    // Java: com/google/devtools/build/skyframe/Version.java:76 Version.constant
    pub fn constant() -> Version {
        Version::Constant
    }
}

impl From<IntVersion> for Version {
    fn from(v: IntVersion) -> Self {
        Version::Int(v)
    }
}

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // PORT: Java prints Object.toString ("...Version$MinimalVersion@<identity hash>"),
            // which is not reproducible; the class name is printed.
            Version::Minimal => f.write_str("MinimalVersion"),
            Version::Constant => f.write_str("ConstantVersion"),
            Version::Int(v) => fmt::Debug::fmt(v, f),
        }
    }
}

/// Versioning scheme based on integers.
///
/// PORT: Java interns instances (`BlazeInterners.newWeakInterner`); equality is by value either
/// way, so this is a plain `Copy` value.
// Java: com/google/devtools/build/skyframe/IntVersion.java:20 IntVersion
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IntVersion {
    val: i64,
}

impl IntVersion {
    // Java: com/google/devtools/build/skyframe/IntVersion.java:24 IntVersion.of
    pub fn of(val: i64) -> IntVersion {
        IntVersion { val }
    }

    /// Returns the integer value as a long.
    // Java: com/google/devtools/build/skyframe/IntVersion.java:35 IntVersion.getVal
    pub fn get_val(&self) -> i64 {
        self.val
    }

    // Java: com/google/devtools/build/skyframe/IntVersion.java:39 IntVersion.next
    pub fn next(&self) -> IntVersion {
        IntVersion::of(self.val.wrapping_add(1))
    }

    // Java: com/google/devtools/build/skyframe/IntVersion.java:44 IntVersion.atMost
    pub fn at_most(&self, other: &Version) -> bool {
        let Version::Int(other) = other else {
            return false;
        };
        self.val <= other.val
    }
}

impl fmt::Debug for IntVersion {
    // Java: com/google/devtools/build/skyframe/IntVersion.java:68 IntVersion.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IntVersion: {}", self.val)
    }
}

/// Encapsulates the two versions relevant to a `NodeEntry`: when it was last evaluated, and when
/// its value last changed.
///
/// PORT: Java's interface is implemented by every `Version` (both versions are the same) and by
/// `NodeVersion.ChangePruned`; here the two cases are the two variants.
// Java: com/google/devtools/build/skyframe/NodeVersion.java:22 NodeVersion
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeVersion {
    /// A single `Version`, both last changed and last evaluated.
    Version(Version),
    /// Basic implementation of `NodeVersion` for the case where `lastChanged` and
    /// `lastEvaluated` are different versions.
    // Java: com/google/devtools/build/skyframe/NodeVersion.java:53 NodeVersion.ChangePruned
    ChangePruned {
        last_changed: Version,
        last_evaluated: Version,
    },
}

impl NodeVersion {
    /// Returns the last version at which a node's value changed.
    ///
    /// In `NodeEntry.setValue` it may be determined that the value being set is the same as the
    /// already-stored value. In that case, the last changed version will remain the same.
    // Java: com/google/devtools/build/skyframe/NodeVersion.java:30 NodeVersion.lastChanged
    pub fn last_changed(&self) -> &Version {
        match self {
            NodeVersion::Version(v) => v.last_changed(),
            NodeVersion::ChangePruned { last_changed, .. } => last_changed,
        }
    }

    /// Returns the last version a `NodeEntry` was evaluated at, even if it re-evaluated to the
    /// same value.
    ///
    /// When a child signals a node with the last version it was changed at in
    /// `NodeEntry.signalDep`, the node need not re-evaluate if the child's version is
    /// [`Version::at_most`] this version, even if `lastChanged` is lower.
    // Java: com/google/devtools/build/skyframe/NodeVersion.java:40 NodeVersion.lastEvaluated
    pub fn last_evaluated(&self) -> &Version {
        match self {
            NodeVersion::Version(v) => v.last_evaluated(),
            NodeVersion::ChangePruned { last_evaluated, .. } => last_evaluated,
        }
    }

    // Java: com/google/devtools/build/skyframe/NodeVersion.java:42 NodeVersion.of
    pub fn of(last_changed: Version, last_evaluated: Version) -> NodeVersion {
        if last_changed == last_evaluated {
            return NodeVersion::Version(last_changed);
        }
        NodeVersion::ChangePruned {
            last_changed,
            last_evaluated,
        }
    }
}

impl From<Version> for NodeVersion {
    fn from(v: Version) -> Self {
        NodeVersion::Version(v)
    }
}

impl fmt::Debug for NodeVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeVersion::Version(v) => fmt::Debug::fmt(v, f),
            // Java: AutoValue toString.
            NodeVersion::ChangePruned {
                last_changed,
                last_evaluated,
            } => write!(
                f,
                "ChangePruned{{lastChanged={last_changed:?}, lastEvaluated={last_evaluated:?}}}"
            ),
        }
    }
}
