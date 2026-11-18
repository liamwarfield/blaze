//! Port of `ResolvedTargets`.

use std::fmt;
use std::hash::Hash;

use indexmap::IndexSet;

/// The result of target pattern evaluation: targets, filtered targets, and an error bit (Java
/// `ResolvedTargets<T>`). Sets keep insertion order.
// Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:31 ResolvedTargets
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTargets<T: Hash + Eq> {
    has_error: bool,
    targets: IndexSet<T>,
    filtered_targets: IndexSet<T>,
}

impl<T: Hash + Eq + Clone> ResolvedTargets<T> {
    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:39 ResolvedTargets.failed
    pub fn failed() -> Self {
        Self::new_with_filtered(IndexSet::new(), IndexSet::new(), true)
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:44 ResolvedTargets.empty
    pub fn empty() -> Self {
        Self::new_with_filtered(IndexSet::new(), IndexSet::new(), false)
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:48 ResolvedTargets.of
    pub fn of(target: T) -> Self {
        Self::new(IndexSet::from([target]), false)
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:56 ResolvedTargets.<init>
    pub fn new_with_filtered(
        targets: IndexSet<T>,
        filtered_targets: IndexSet<T>,
        has_error: bool,
    ) -> Self {
        ResolvedTargets {
            has_error,
            targets,
            filtered_targets,
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:62 ResolvedTargets.<init>
    pub fn new(targets: IndexSet<T>, has_error: bool) -> Self {
        Self::new_with_filtered(targets, IndexSet::new(), has_error)
    }

    pub fn has_error(&self) -> bool {
        self.has_error
    }

    pub fn get_targets(&self) -> &IndexSet<T> {
        &self.targets
    }

    pub fn get_filtered_targets(&self) -> &IndexSet<T> {
        &self.filtered_targets
    }

    // PORT: concurrentBuilder() (concurrent sets) is not ported; use `builder()`.
    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:95 ResolvedTargets.builder
    pub fn builder() -> ResolvedTargetsBuilder<T> {
        ResolvedTargetsBuilder {
            targets: IndexSet::new(),
            filtered_targets: IndexSet::new(),
            has_error: false,
        }
    }
}

// Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:69 ResolvedTargets.toString
impl<T: Hash + Eq + fmt::Display> fmt::Display for ResolvedTargets<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn set<T: fmt::Display>(
            f: &mut fmt::Formatter<'_>,
            s: impl Iterator<Item = T>,
        ) -> fmt::Result {
            f.write_str("[")?;
            for (i, t) in s.enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{}", t)?;
            }
            f.write_str("]")
        }
        f.write_str("ResolvedTargets(")?;
        set(f, self.targets.iter())?;
        f.write_str(", filtered=")?;
        set(f, self.filtered_targets.iter())?;
        write!(f, ", hasError={})", self.has_error)
    }
}

/// Builder for [`ResolvedTargets`] (Java `ResolvedTargets.Builder`).
// Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:99 ResolvedTargets.Builder
#[derive(Debug, Clone)]
pub struct ResolvedTargetsBuilder<T: Hash + Eq> {
    targets: IndexSet<T>,
    filtered_targets: IndexSet<T>,
    has_error: bool,
}

impl<T: Hash + Eq + Clone> ResolvedTargetsBuilder<T> {
    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:113 ResolvedTargets.Builder.build
    pub fn build(&self) -> ResolvedTargets<T> {
        ResolvedTargets::new_with_filtered(
            self.targets.clone(),
            self.filtered_targets.clone(),
            self.has_error,
        )
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:118 ResolvedTargets.Builder.merge
    pub fn merge(&mut self, other: &ResolvedTargets<T>) -> &mut Self {
        self.remove_all(other.filtered_targets.iter().cloned());
        self.add_all(other.targets.iter().cloned());
        if other.has_error {
            self.has_error = true;
        }
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:128 ResolvedTargets.Builder.add
    pub fn add(&mut self, target: T) -> &mut Self {
        self.filtered_targets.shift_remove(&target);
        self.targets.insert(target);
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:135 ResolvedTargets.Builder.addAll
    pub fn add_all(&mut self, targets: impl IntoIterator<Item = T>) -> &mut Self {
        let targets: Vec<T> = targets.into_iter().collect();
        for t in &targets {
            self.targets.insert(t.clone());
        }
        for t in &targets {
            self.filtered_targets.shift_remove(t);
        }
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:141 ResolvedTargets.Builder.remove
    pub fn remove(&mut self, target: T) {
        self.targets.shift_remove(&target);
        self.filtered_targets.insert(target);
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:147 ResolvedTargets.Builder.removeAll
    pub fn remove_all(&mut self, targets: impl IntoIterator<Item = T>) -> &mut Self {
        let targets: Vec<T> = targets.into_iter().collect();
        for t in &targets {
            self.filtered_targets.insert(t.clone());
        }
        for t in &targets {
            self.targets.shift_remove(t);
        }
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:154 ResolvedTargets.Builder.filter
    pub fn filter(&mut self, predicate: impl Fn(&T) -> bool) -> &mut Self {
        let old_targets = std::mem::take(&mut self.targets);
        for target in old_targets {
            if predicate(&target) {
                self.add(target);
            } else {
                self.remove(target);
            }
        }
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:168 ResolvedTargets.Builder.setError
    pub fn set_error(&mut self) -> &mut Self {
        self.has_error = true;
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:174 ResolvedTargets.Builder.mergeError
    pub fn merge_error(&mut self, has_error: bool) -> &mut Self {
        self.has_error |= has_error;
        self
    }

    // Java: com/google/devtools/build/lib/cmdline/ResolvedTargets.java:179 ResolvedTargets.Builder.isEmpty
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}
