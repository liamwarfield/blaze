//! Port of `packages/RuleClassId.java`.

/// A rule class's name and unique key (Java `RuleClassId`).
// Java: com/google/devtools/build/lib/packages/RuleClassId.java:27 RuleClassId
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleClassId {
    pub name: String,
    pub key: String,
}

impl RuleClassId {
    // Java: com/google/devtools/build/lib/packages/RuleClassId.java:33 RuleClassId.create
    pub fn create(name: &str, key: &str) -> Self {
        RuleClassId {
            name: name.to_owned(),
            key: key.to_owned(),
        }
    }
}
