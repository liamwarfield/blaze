//! `OptionPriority`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use crate::exception::OptionsParsingException;

// Java: com/google/devtools/common/options/OptionPriority.java:129 OptionPriority.PriorityCategory
/// The priority of option values, in order of increasing priority.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PriorityCategory {
    DEFAULT,
    COMPUTED_DEFAULT,
    RC_FILE,
    COMMAND_LINE,
    INVOCATION_POLICY,
    SOFTWARE_REQUIREMENT,
}

impl PriorityCategory {
    pub const VALUES: [PriorityCategory; 6] = [
        PriorityCategory::DEFAULT,
        PriorityCategory::COMPUTED_DEFAULT,
        PriorityCategory::RC_FILE,
        PriorityCategory::COMMAND_LINE,
        PriorityCategory::INVOCATION_POLICY,
        PriorityCategory::SOFTWARE_REQUIREMENT,
    ];
}

impl fmt::Display for PriorityCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

struct Inner {
    priority_category: PriorityCategory,
    priority_indices: Vec<i32>,
    already_expanded: AtomicBool,
}

// Java: com/google/devtools/common/options/OptionPriority.java:28 OptionPriority
/// The position of an option in the interpretation order: category first, then the index path.
///
/// This is a shared handle, like the Java object reference: clones share the "already expanded"
/// flag that `getChildPriority` sets.
#[derive(Clone)]
pub struct OptionPriority(Arc<Inner>);

impl OptionPriority {
    fn create(priority_category: PriorityCategory, priority_indices: Vec<i32>) -> OptionPriority {
        OptionPriority(Arc::new(Inner {
            priority_category,
            priority_indices,
            already_expanded: AtomicBool::new(false),
        }))
    }

    // Java: com/google/devtools/common/options/OptionPriority.java:48 OptionPriority.lowestOptionPriorityAtCategory
    pub fn lowest_option_priority_at_category(category: PriorityCategory) -> OptionPriority {
        Self::create(category, vec![0])
    }

    // Java: com/google/devtools/common/options/OptionPriority.java:57 OptionPriority.nextOptionPriority
    pub fn next_option_priority(priority: &OptionPriority) -> OptionPriority {
        let mut indices = priority.0.priority_indices.clone();
        *indices.last_mut().unwrap() += 1;
        Self::create(priority.0.priority_category, indices)
    }

    // Java: com/google/devtools/common/options/OptionPriority.java:74 OptionPriority.getChildPriority
    pub fn get_child_priority(
        parent_priority: &OptionPriority,
    ) -> Result<OptionPriority, OptionsParsingException> {
        if parent_priority
            .0
            .already_expanded
            .swap(true, AtomicOrdering::SeqCst)
        {
            return Err(OptionsParsingException::new(
                "Tried to expand option too many times",
            ));
        }
        let mut indices = parent_priority.0.priority_indices.clone();
        indices.push(0);
        Ok(Self::create(parent_priority.0.priority_category, indices))
    }

    // Java: com/google/devtools/common/options/OptionPriority.java:88 OptionPriority.getPriorityCategory
    pub fn get_priority_category(&self) -> PriorityCategory {
        self.0.priority_category
    }

    pub fn priority_indices(&self) -> &[i32] {
        &self.0.priority_indices
    }
}

impl Ord for OptionPriority {
    // Java: com/google/devtools/common/options/OptionPriority.java:93 OptionPriority.compareTo
    fn cmp(&self, o: &Self) -> Ordering {
        if self.0.priority_category == o.0.priority_category {
            let a = &self.0.priority_indices;
            let b = &o.0.priority_indices;
            for i in 0..a.len().min(b.len()) {
                if a[i] != b[i] {
                    return a[i].cmp(&b[i]);
                }
            }
            return a.len().cmp(&b.len());
        }
        self.0.priority_category.cmp(&o.0.priority_category)
    }
}

impl PartialOrd for OptionPriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for OptionPriority {
    // Java: com/google/devtools/common/options/OptionPriority.java:108 OptionPriority.equals
    fn eq(&self, other: &Self) -> bool {
        self.0.priority_category == other.0.priority_category
            && self.0.priority_indices == other.0.priority_indices
    }
}
impl Eq for OptionPriority {}

impl Hash for OptionPriority {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.priority_category.hash(state);
        self.0.priority_indices.hash(state);
    }
}

impl fmt::Display for OptionPriority {
    // Java: com/google/devtools/common/options/OptionPriority.java:122 OptionPriority.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let indices: Vec<String> = self
            .0
            .priority_indices
            .iter()
            .map(|i| i.to_string())
            .collect();
        write!(
            f,
            "OptionPriority({},[{}])",
            self.0.priority_category,
            indices.join(", ")
        )
    }
}

impl fmt::Debug for OptionPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
