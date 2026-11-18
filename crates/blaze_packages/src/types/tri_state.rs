//! Port of `packages/TriState.java`.

/// Java `TriState`: like a boolean attribute whose default value can be distinguished from both
/// explicitly assigned values.
// Java: com/google/devtools/build/lib/packages/TriState.java:20 TriState
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TriState {
    Yes,
    No,
    Auto,
}

impl TriState {
    // Java: com/google/devtools/build/lib/packages/TriState.java:25 TriState.toInt
    pub fn to_int(self) -> i32 {
        match self {
            TriState::Yes => 1,
            TriState::No => 0,
            TriState::Auto => -1,
        }
    }

    // Java: com/google/devtools/build/lib/packages/TriState.java:33 TriState.fromInt
    pub fn from_int(n: i32) -> TriState {
        match n {
            1 => TriState::Yes,
            0 => TriState::No,
            -1 => TriState::Auto,
            _ => panic!("IllegalArgumentException: TriState must be -1, 0, or 1"),
        }
    }
}
