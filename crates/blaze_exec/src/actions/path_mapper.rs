//! `lib/actions/PathMapper.java` as `Spawn.getPathMapper()` returns it: `PathMapper.NOOP`, or the
//! action's mapper (`StrippingPathMapper` under `--experimental_output_paths=strip`), which lives
//! in `blaze_actions` with the command lines it maps.

use std::fmt;
use std::sync::Arc;

use blaze_actions::actions::path_mapper::PathMapper as ActionsPathMapper;
use blaze_vfs::PathFragment;

/// Maps the exec paths of a spawn's command line, inputs and outputs.
// Java: com/google/devtools/build/lib/actions/PathMapper.java:61 PathMapper
#[derive(Clone)]
pub enum PathMapper {
    /// `PathMapper.NOOP`.
    Noop,
    /// A mapper that is not the no-op (`PathMappers.create` of the action that made the spawn).
    Mapped(Arc<dyn ActionsPathMapper>),
}

impl PathMapper {
    /// `PathMapper.NOOP`.
    pub const NOOP: PathMapper = PathMapper::Noop;

    /// The spawn's view of an action's path mapper (`PathMappers.create`): `NOOP` when it maps
    /// nothing.
    pub fn of(mapper: Arc<dyn ActionsPathMapper>) -> PathMapper {
        if mapper.is_noop() {
            PathMapper::Noop
        } else {
            PathMapper::Mapped(mapper)
        }
    }

    /// The action-level mapper (`PathMapper.NOOP` for `Noop`).
    pub fn as_actions_path_mapper(&self) -> &dyn ActionsPathMapper {
        match self {
            PathMapper::Noop => &blaze_actions::actions::path_mapper::NOOP,
            PathMapper::Mapped(m) => m.as_ref(),
        }
    }

    /// `map(PathFragment execPath)`.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:102 PathMapper.map
    pub fn map(&self, exec_path: &PathFragment) -> PathFragment {
        match self {
            PathMapper::Noop => exec_path.clone(),
            PathMapper::Mapped(m) => m.map(exec_path),
        }
    }

    /// `isNoop()`.
    // Java: com/google/devtools/build/lib/actions/PathMapper.java:185 PathMapper.isNoop
    pub fn is_noop(&self) -> bool {
        match self {
            PathMapper::Noop => true,
            PathMapper::Mapped(m) => m.is_noop(),
        }
    }
}

impl fmt::Debug for PathMapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathMapper::Noop => f.write_str("PathMapper.NOOP"),
            PathMapper::Mapped(m) => write!(f, "PathMapper({})", m.cache_key()),
        }
    }
}

/// Identity, as Java compares mapper objects.
impl PartialEq for PathMapper {
    fn eq(&self, other: &PathMapper) -> bool {
        match (self, other) {
            (PathMapper::Noop, PathMapper::Noop) => true,
            (PathMapper::Mapped(a), PathMapper::Mapped(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for PathMapper {}
