//! Port of the `PathMapper.NOOP` part of `lib/actions/PathMapper.java`.

use blaze_vfs::PathFragment;

/// Maps exec paths of a spawn (path mapping, `--experimental_output_paths=strip`). Only the
/// no-op mapper is ported; Blaze does not support path mapping yet.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:61 PathMapper
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathMapper {
    /// `PathMapper.NOOP`.
    Noop,
}

impl PathMapper {
    /// `PathMapper.NOOP`.
    pub const NOOP: PathMapper = PathMapper::Noop;

    /// `map(PathFragment execPath)`.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:102 PathMapper.map
    pub fn map(&self, exec_path: &PathFragment) -> PathFragment {
        match self {
            PathMapper::Noop => exec_path.clone(),
        }
    }

    /// `isNoop()`.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:185 PathMapper.isNoop
    pub fn is_noop(&self) -> bool {
        matches!(self, PathMapper::Noop)
    }
}
