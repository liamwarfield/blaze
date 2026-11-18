//! Port of `lib/analysis/actions/Substitution.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A substitution with a fixed key, and a computed value. The computed value must not change over
/// the lifetime of an instance, though the [`ComputedSubstitution::get_value`] method may return
/// different String objects.
///
/// It should be assumed that the `get_key` invocation is cheap, and that the `get_value`
/// invocation is expensive.
// Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:141 Substitution.ComputedSubstitution
pub trait ComputedSubstitution: Send + Sync + fmt::Debug {
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:150 ComputedSubstitution.getKey
    fn get_key(&self) -> &str;

    /// `Err` is the `EvalException` message.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:40 Substitution.getValue
    fn get_value(&self) -> Result<String, String>;
}

/// A pair of a string to be substituted and a string to substitute it with. For simplicity, these
/// are called key and value. All implementations must be immutable, and always return the
/// identical key. The returned values must be the same, though they need not be the same object.
///
/// It should be assumed that the [`Substitution::get_key`] invocation is cheap, and that the
/// [`Substitution::get_value`] invocation is expensive.
// Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:35 Substitution
#[derive(Clone, Debug)]
pub enum Substitution {
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:53 Substitution.StringSubstitution
    String {
        key: String,
        value: String,
    },
    /// The values are joined by spaces before substitution.
    ///
    /// PORT: Java holds an `ImmutableList<?>` and joins the elements' `toString()`; callers pass
    /// the already-stringified elements.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:73 Substitution.ListSubstitution
    List {
        key: String,
        value: Vec<String>,
    },
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:141 Substitution.ComputedSubstitution
    Computed(Arc<dyn ComputedSubstitution>),
}

impl Substitution {
    /// Returns an immutable Substitution instance for the given key and value.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:94 Substitution.of
    pub fn of(key: impl Into<String>, value: impl Into<String>) -> Substitution {
        Substitution::String {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Returns an immutable Substitution instance for the key and list of values. The values will
    /// be joined by spaces before substitution.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:104 Substitution.ofSpaceSeparatedList
    pub fn of_space_separated_list(key: impl Into<String>, value: Vec<String>) -> Substitution {
        Substitution::List {
            key: key.into(),
            value,
        }
    }

    /// A substitution whose value is computed on demand.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:144 ComputedSubstitution.<init>
    pub fn computed(c: Arc<dyn ComputedSubstitution>) -> Substitution {
        Substitution::Computed(c)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:38 Substitution.getKey
    pub fn get_key(&self) -> &str {
        match self {
            // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:63 StringSubstitution.getKey
            Substitution::String { key, .. } => key,
            // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:83 ListSubstitution.getKey
            Substitution::List { key, .. } => key,
            Substitution::Computed(c) => c.get_key(),
        }
    }

    /// `Err` is the `EvalException` message.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:40 Substitution.getValue
    pub fn get_value(&self) -> Result<String, String> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:68 StringSubstitution.getValue
            Substitution::String { value, .. } => Ok(value.clone()),
            // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:88 ListSubstitution.getValue
            Substitution::List { value, .. } => Ok(value.join(" ")),
            Substitution::Computed(c) => c.get_value(),
        }
    }

    /// Not intended for use in production code.
    // Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:45 Substitution.getValueUnchecked
    pub fn get_value_unchecked(&self) -> String {
        match self.get_value() {
            Ok(v) => v,
            Err(e) => panic!("IllegalStateException: {}", e),
        }
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:112 Substitution.equals
impl PartialEq for Substitution {
    fn eq(&self, other: &Substitution) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        other.get_key() == self.get_key()
            && other.get_value_unchecked() == self.get_value_unchecked()
    }
}

impl Eq for Substitution {}

// Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:124 Substitution.hashCode
impl Hash for Substitution {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get_key().hash(state);
        self.get_value_unchecked().hash(state);
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/Substitution.java:129 Substitution.toString
impl fmt::Display for Substitution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Substitution({} -> {})",
            self.get_key(),
            self.get_value_unchecked()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fixed;

    impl ComputedSubstitution for Fixed {
        fn get_key(&self) -> &str {
            "%k%"
        }
        fn get_value(&self) -> Result<String, String> {
            Ok("v".to_string())
        }
    }

    #[test]
    fn values_and_equality() {
        let a = Substitution::of("%k%", "v");
        let b = Substitution::of_space_separated_list("%k%", vec!["v".to_string()]);
        let c = Substitution::computed(Arc::new(Fixed));
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_eq!(
            Substitution::of_space_separated_list("k", vec!["a".into(), "b".into()])
                .get_value_unchecked(),
            "a b"
        );
        assert_eq!(a.to_string(), "Substitution(%k% -> v)");
    }
}
