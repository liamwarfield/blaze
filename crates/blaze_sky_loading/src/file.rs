//! FILE: `FileFunction` and `FileKey`.
//!
//! Java: com/google/devtools/build/lib/skyframe/{FileFunction,FileKey,CycleUtils}.java.
//!
//! Most of the complexity in the implementation results from wanting incremental correctness in
//! the presence of symlinks, esp. ancestor directory symlinks. Blaze keeps the same algorithm
//! and the same dependencies so that symlink errors and real paths match Bazel.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
};
use blaze_vfs::{Path, PathFragment, Root, RootedPath};
use parking_lot::RwLock;

use crate::actions::{FileStateType, FileStateValue, FileValue};
use crate::directories::LoadingDirectories;
use crate::file_chain_uniqueness::{
    FileSymlinkCycleUniquenessKey, FileSymlinkInfiniteExpansionUniquenessKey,
};
use crate::file_state::FileStateKey;
use crate::io::IOException;
use crate::pkgcache::PathPackageLocator;
use crate::sky_functions;

/// Key for [`FileFunction`]. `Debug` prints `FILE:<rooted path>`.
// PORT: skipped, a newtype key (Key::FUNCTION; no interning): FileKey.create, FileKey.intern, FileKey.functionName, FileKey.getSkyKeyInterner
// Java: com/google/devtools/build/lib/skyframe/FileKey.java:28 FileKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileKey(pub RootedPath);

impl FileKey {
    /// Returns a key for building a file value for the given root-relative path.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:146 FileValue.key
    pub fn key(rooted_path: RootedPath) -> FileKey {
        FileKey(rooted_path)
    }

    pub fn argument(&self) -> &RootedPath {
        &self.0
    }
}

impl fmt::Debug for FileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for FileKey {
    type Value = FileValue;
    const FUNCTION: SkyFunctionName = sky_functions::FILE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:53 SkyFunctions.FILE (createSemiHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::SemiHermetic;
    /// Java: the `FileValue` subclasses' `equals` over the same fields. PORT: Java's `SymlinkFileValueWithStoredChain.equals` uses `instanceof`, so an old value of that class equals a new `SymlinkFileValueWithUnboundedAncestorExpansion` with the same fields; here variants never compare equal (less pruning, never wrong).
    // Java: com/google/devtools/build/lib/actions/FileValue.java:294 FileValue.DifferentRealPathFileValueWithUnboundedAncestorExpansion.equals
    fn values_equal(old: &FileValue, new: &FileValue) -> bool {
        old == new
    }
}

/// Separates path and cycle from a combined list: elements before the first one matching
/// `start_of_cycle`, and the rest.
// PORT: skipped, the private constructor of a static-only class: CycleUtils.<init>
// Java: com/google/devtools/build/lib/skyframe/CycleUtils.java:27 CycleUtils.splitIntoPathAndChain
pub(crate) fn split_into_path_and_chain<S: Clone>(
    start_of_cycle: impl Fn(&S) -> bool,
    path_and_cycle: &[S],
) -> (Vec<S>, Vec<S>) {
    let mut in_path_to_cycle = true;
    let mut path_to_cycle = Vec::new();
    let mut cycle = Vec::new();
    for elt in path_and_cycle {
        if start_of_cycle(elt) {
            in_path_to_cycle = false;
        }
        if in_path_to_cycle {
            path_to_cycle.push(elt.clone());
        } else {
            cycle.push(elt.clone());
        }
    }
    (path_to_cycle, cycle)
}

// Java: com/google/devtools/build/lib/skyframe/FileFunction.java:76 FileFunction.SymlinkResolutionState
struct SymlinkResolutionState {
    // Suppose we have a path p. One of the goals of FileFunction is to resolve the "real path",
    // if any, of p. The basic algorithm is to use the fully resolved path of p's parent directory
    // to determine the fully resolved path of p. This is complicated when symlinks are involved,
    // and is especially complicated when ancestor directory symlinks are involved.
    //
    // Since FileStateValues are the roots of invalidation, care has to be taken to ensuring we
    // declare the proper FileStateValue deps. As a concrete example, let p = a/b and imagine (i)
    // a is a direct symlink to c and also (ii) c/b is an existing file. Among other direct deps,
    // we want to have a direct dep on FileStateValue(c/b), since that's the node that will be
    // changed if the actual contents of a/b (aka c/b) changes. To rephrase: a dep on
    // FileStateValue(a/b) won't do anything productive since that path will never be in the
    // Skyframe diff.
    //
    // In the course of resolving the real path of p, there will be a logical chain of paths we
    // consider. Going with the example from above, the full chain of paths we consider is
    // [a/b, c/b].
    logical_chain: Vec<RootedPath>,
    // Same contents as 'logicalChain', except stored as a sorted set for efficiency reasons.
    // See the usage in checkPathSeenDuringPartialResolutionInternal.
    sorted_logical_chain: BTreeSet<Path>,
    path_to_unbounded_ancestor_symlink_expansion_chain: Option<Vec<RootedPath>>,
    unbounded_ancestor_symlink_expansion_chain: Option<Vec<RootedPath>>,
}

impl SymlinkResolutionState {
    fn new() -> Self {
        SymlinkResolutionState {
            logical_chain: Vec::new(),
            sorted_logical_chain: BTreeSet::new(),
            path_to_unbounded_ancestor_symlink_expansion_chain: None,
            unbounded_ancestor_symlink_expansion_chain: None,
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/FileFunction.java:280 FileFunction.PartialResolutionResult
struct PartialResolutionResult {
    rooted_path: RootedPath,
    file_state_value: Arc<FileStateValue>,
}

/// The result of a step: `Ok(Some(_))` done, `Ok(None)` a dependency was missing, `Err` the
/// function throws (Java `FileFunctionException`) or was interrupted.
type StepResult<T> = Result<Option<T>, ComputeError>;

fn file_function_exception(e: IOException) -> ComputeError {
    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:400 FileFunction.FileFunctionException
    SkyFunctionException::persistent(e).into()
}

/// A SkyFunction for [`FileValue`]s.
// Java: com/google/devtools/build/lib/skyframe/FileFunction.java:61 FileFunction
pub struct FileFunction {
    pkg_locator: Arc<RwLock<PathPackageLocator>>,
    immutable_paths: Vec<Root>,
}

impl FileFunction {
    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:65 FileFunction.<init>
    pub fn new(
        pkg_locator: Arc<RwLock<PathPackageLocator>>,
        directories: &LoadingDirectories,
    ) -> FileFunction {
        FileFunction {
            pkg_locator,
            immutable_paths: vec![
                Root::from_path(directories.get_output_base().clone()),
                Root::from_path(directories.get_install_base().clone()),
            ],
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:101 FileFunction.compute
    pub fn compute(
        &self,
        sky_key: &FileKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<FileValue>, ComputeError> {
        let rooted_path = sky_key.argument();
        let mut symlink_resolution_state = SymlinkResolutionState::new();

        // Fully resolve the path of the parent directory, but only if the current file is not the
        // filesystem root (has no parent) or a package path root (treated opaquely and handled by
        // skyframe's DiffAwareness interface).
        //
        // This entails resolving ancestor symlinks fully. Note that this is the first thing we do
        // - if an ancestor is part of a symlink cycle, we want to detect that quickly as it gives
        // a more informative error message than we'd get doing bogus filesystem operations.
        let Some(resolve_from_ancestors_result) =
            Self::resolve_from_ancestors(rooted_path, &mut symlink_resolution_state, env)?
        else {
            return Ok(None);
        };
        let rooted_path_from_ancestors = resolve_from_ancestors_result.rooted_path;
        let file_state_value_from_ancestors = resolve_from_ancestors_result.file_state_value;
        if file_state_value_from_ancestors.get_type() == FileStateType::Nonexistent {
            return Ok(Some(FileValue::value(
                symlink_resolution_state.logical_chain,
                symlink_resolution_state.path_to_unbounded_ancestor_symlink_expansion_chain,
                symlink_resolution_state.unbounded_ancestor_symlink_expansion_chain,
                rooted_path,
                FileStateValue::NONEXISTENT_FILE_STATE_NODE,
                rooted_path_from_ancestors,
                (*file_state_value_from_ancestors).clone(),
            )));
        }

        let mut real_rooted_path = rooted_path_from_ancestors;
        let mut real_file_state_value = file_state_value_from_ancestors.clone();

        while real_file_state_value.get_type().is_symlink() {
            let Some(get_symlink_target_rooted_path_result) = self.get_symlink_target_rooted_path(
                &real_rooted_path,
                real_file_state_value.get_symlink_target(),
                &mut symlink_resolution_state,
                env,
            )?
            else {
                return Ok(None);
            };
            real_rooted_path = get_symlink_target_rooted_path_result.rooted_path;
            real_file_state_value = get_symlink_target_rooted_path_result.file_state_value;
        }

        Ok(Some(FileValue::value(
            symlink_resolution_state.logical_chain,
            symlink_resolution_state.path_to_unbounded_ancestor_symlink_expansion_chain,
            symlink_resolution_state.unbounded_ancestor_symlink_expansion_chain,
            rooted_path,
            (*file_state_value_from_ancestors).clone(),
            real_rooted_path,
            (*real_file_state_value).clone(),
        )))
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:161 FileFunction.getChild
    fn get_child(
        parent: &RootedPath,
        base_name: &str,
        original_parent: &RootedPath,
        original_child: &RootedPath,
    ) -> RootedPath {
        if parent == original_parent {
            return original_child.clone(); // Avoid constructing a new instance if we already have the child.
        }
        RootedPath::to_rooted_path(
            parent.get_root().clone(),
            &parent.get_root_relative_path().get_child(base_name),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:170 FileFunction.toRootedPath
    fn to_rooted_path(&self, path: &Path) -> RootedPath {
        // We check whether the path to be transformed is under the output base or the install
        // base. These directories are under the control of Bazel and it therefore does not make
        // much sense to check for changes in them or in their ancestors in the usual Skyframe
        // way.
        let pkg_locator = self.pkg_locator.read().clone();
        RootedPath::to_rooted_path_maybe_under_root(
            path,
            pkg_locator
                .get_path_entries()
                .iter()
                .chain(self.immutable_paths.iter()),
        )
    }

    /// Returns the path and file state of `rooted_path`, accounting for ancestor symlinks, or
    /// `None` if there was a missing dep.
    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:183 FileFunction.resolveFromAncestors
    fn resolve_from_ancestors(
        rooted_path: &RootedPath,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> StepResult<PartialResolutionResult> {
        match rooted_path.get_parent_directory() {
            Some(parent_rooted_path) => Self::resolve_from_ancestors_with_parent(
                rooted_path,
                &parent_rooted_path,
                symlink_resolution_state,
                env,
            ),
            None => {
                Self::resolve_from_ancestors_no_parent(rooted_path, symlink_resolution_state, env)
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:193 FileFunction.resolveFromAncestorsWithParent
    fn resolve_from_ancestors_with_parent(
        rooted_path: &RootedPath,
        parent_rooted_path: &RootedPath,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> StepResult<PartialResolutionResult> {
        let relative_path = rooted_path.get_root_relative_path();
        let base_name = relative_path.get_base_name();

        let Some(parent_file_value) = env.get_value(FileKey::key(parent_rooted_path.clone()))?
        else {
            return Ok(None);
        };

        let rooted_path_from_ancestors = Self::get_child(
            &parent_file_value.real_rooted_path(parent_rooted_path),
            base_name,
            parent_rooted_path,
            rooted_path,
        );

        if !parent_file_value.exists() || !parent_file_value.is_directory() {
            return Ok(Some(PartialResolutionResult {
                rooted_path: rooted_path_from_ancestors,
                file_state_value: Arc::new(FileStateValue::NONEXISTENT_FILE_STATE_NODE),
            }));
        }

        for parent_partial_rooted_path in
            parent_file_value.logical_chain_during_resolution(parent_rooted_path)
        {
            Self::check_and_note_path_seen_during_partial_resolution(
                &Self::get_child(
                    &parent_partial_rooted_path,
                    base_name,
                    parent_rooted_path,
                    rooted_path,
                ),
                symlink_resolution_state,
                env,
            )?;
            if env.values_missing() {
                return Ok(None);
            }
        }

        let Some(file_state_value_from_ancestors) =
            env.get_value(FileStateKey::key(rooted_path_from_ancestors.clone()))?
        else {
            return Ok(None);
        };

        Ok(Some(PartialResolutionResult {
            rooted_path: rooted_path_from_ancestors,
            file_state_value: file_state_value_from_ancestors,
        }))
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:243 FileFunction.resolveFromAncestorsNoParent
    fn resolve_from_ancestors_no_parent(
        rooted_path: &RootedPath,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> StepResult<PartialResolutionResult> {
        Self::check_and_note_path_seen_during_partial_resolution(
            rooted_path,
            symlink_resolution_state,
            env,
        )?;
        if env.values_missing() {
            return Ok(None);
        }
        let Some(real_file_state_value) = env.get_value(FileStateKey::key(rooted_path.clone()))?
        else {
            return Ok(None);
        };
        Ok(Some(PartialResolutionResult {
            rooted_path: rooted_path.clone(),
            file_state_value: real_file_state_value,
        }))
    }

    /// Returns the symlink target and file state of `rooted_path`'s symlink to `symlink_target`,
    /// accounting for ancestor symlinks, or `None` if there was a missing dep.
    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:293 FileFunction.getSymlinkTargetRootedPath
    fn get_symlink_target_rooted_path(
        &self,
        rooted_path: &RootedPath,
        symlink_target: &PathFragment,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> StepResult<PartialResolutionResult> {
        let path = rooted_path.as_path();
        let symlink_target_path = if symlink_target.is_absolute() {
            path.get_relative(symlink_target)
        } else {
            match path.get_parent_directory() {
                Some(parent_path) => parent_path.get_relative(symlink_target),
                None => path.get_relative(symlink_target),
            }
        };
        let symlink_target_rooted_path = self.to_rooted_path(&symlink_target_path);
        Self::check_path_seen_during_partial_resolution(
            &symlink_target_rooted_path,
            symlink_resolution_state,
            env,
        )?;
        if env.values_missing() {
            return Ok(None);
        }
        // The symlink target could have a different parent directory, which itself could be a
        // directory symlink (or have an ancestor directory symlink)!
        Self::resolve_from_ancestors(&symlink_target_rooted_path, symlink_resolution_state, env)
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:322 FileFunction.checkAndNotePathSeenDuringPartialResolution
    fn check_and_note_path_seen_during_partial_resolution(
        rooted_path: &RootedPath,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> Result<(), ComputeError> {
        let path = rooted_path.as_path();
        Self::check_path_seen_during_partial_resolution_internal(
            rooted_path,
            &path,
            symlink_resolution_state,
            env,
        )?;
        symlink_resolution_state.sorted_logical_chain.insert(path);
        symlink_resolution_state
            .logical_chain
            .push(rooted_path.clone());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:331 FileFunction.checkPathSeenDuringPartialResolution
    fn check_path_seen_during_partial_resolution(
        rooted_path: &RootedPath,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> Result<(), ComputeError> {
        Self::check_path_seen_during_partial_resolution_internal(
            rooted_path,
            &rooted_path.as_path(),
            symlink_resolution_state,
            env,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/FileFunction.java:337 FileFunction.checkPathSeenDuringPartialResolutionInternal
    fn check_path_seen_during_partial_resolution_internal(
        rooted_path: &RootedPath,
        path: &Path,
        symlink_resolution_state: &mut SymlinkResolutionState,
        env: &mut Environment<'_>,
    ) -> Result<(), ComputeError> {
        // We are about to perform another step of partial real path resolution. 'logicalChain' is
        // the chain of paths we've considered so far, and 'rootedPath' / 'path' is the proposed
        // next path we consider.
        //
        // There are three interesting cases to consider, all stemming from symlinks:
        //   (i) Symlink cycle:
        //     p -> p1 -> p2 -> p1
        //     This means `p` has no real path, so we error out.
        //   (ii) Unbounded expansion caused by a symlink to a descendant of a member of the chain:
        //     p -> a/b -> c/d -> a/b/e
        //     This means `p` has no real path, so we error out.
        //   (iii) Unbounded expansion caused by a symlink to an ancestor of a member of the
        //   chain:
        //     p -> a/b -> c/d -> a
        //     This is not necessarily a problem (the real path of `p` in this example is simply
        //     `a`), so we just note the unbounded ancestor expansion and let consumers decide what
        //     to do.
        //
        // We can detect all three of these symlink issues via inspection of the proposed new
        // element. Here is our incremental algorithm:
        //   If 'path' is in 'sortedLogicalChain' then we have a found a cycle (i).
        //   If 'path' is a descendant of any path p in 'sortedLogicalChain' then we have
        //   unbounded expansion (ii).
        //   If 'path' is an ancestor of any path p in 'sortedLogicalChain' then we have unbounded
        //   expansion (iii).
        // We can check for these cases efficiently (read: sublinear time) by finding the extremal
        // candidate p for (ii) and (iii).
        let sorted = &symlink_resolution_state.sorted_logical_chain;
        let seen_floor_path: Option<Path> = sorted.range(..=path.clone()).next_back().cloned();
        let seen_ceiling_path: Option<Path> = sorted.range(path.clone()..).next().cloned();
        enum Uniqueness {
            Cycle(FileSymlinkCycleUniquenessKey),
            InfiniteExpansion(FileSymlinkInfiniteExpansionUniquenessKey),
        }
        let mut uniqueness: Option<(Uniqueness, IOException)> = None;
        if sorted.contains(path) {
            // 'rootedPath' is [transitively] a symlink to a previous element in the symlink chain
            // (i).
            let (path_to_cycle, cycle) = split_into_path_and_chain(
                is_path_predicate(path),
                &symlink_resolution_state.logical_chain,
            );
            let key = FileSymlinkCycleUniquenessKey::key(&cycle);
            let fsce = IOException::file_symlink_cycle(path_to_cycle, cycle);
            uniqueness = Some((Uniqueness::Cycle(key), fsce));
        } else if let Some(seen_floor_path) =
            seen_floor_path.filter(|floor| path.starts_with(floor))
        {
            // 'rootedPath' is [transitively] a symlink to a descendant of a previous element in
            // the symlink chain (ii).
            let mut chain = symlink_resolution_state.logical_chain.clone();
            chain.push(rooted_path.clone());
            let (path_to_chain, chain) =
                split_into_path_and_chain(is_path_predicate(&seen_floor_path), &chain);
            let key = FileSymlinkInfiniteExpansionUniquenessKey::key(&chain);
            let fse = IOException::file_symlink_infinite_expansion(path_to_chain, chain);
            uniqueness = Some((Uniqueness::InfiniteExpansion(key), fse));
        } else if let Some(seen_ceiling_path) =
            seen_ceiling_path.filter(|ceiling| ceiling.starts_with(path))
        {
            // 'rootedPath' is [transitively] a symlink to an ancestor of a previous element in
            // the symlink chain (iii).
            if symlink_resolution_state
                .unbounded_ancestor_symlink_expansion_chain
                .is_none()
            {
                let mut chain = symlink_resolution_state.logical_chain.clone();
                chain.push(rooted_path.clone());
                let (path_to_chain, chain) =
                    split_into_path_and_chain(is_path_predicate(&seen_ceiling_path), &chain);
                symlink_resolution_state.path_to_unbounded_ancestor_symlink_expansion_chain =
                    Some(path_to_chain);
                symlink_resolution_state.unbounded_ancestor_symlink_expansion_chain = Some(chain);
            }
        }
        if let Some((key, fse)) = uniqueness {
            // Note that this dependency is merely to ensure that each unique symlink error gets
            // reported exactly once.
            match key {
                Uniqueness::Cycle(k) => {
                    env.get_value(k)?;
                }
                Uniqueness::InfiniteExpansion(k) => {
                    env.get_value(k)?;
                }
            }
            if env.values_missing() {
                return Ok(());
            }
            return Err(file_function_exception(fse));
        }
        Ok(())
    }
}

// Java: com/google/devtools/build/lib/skyframe/FileFunction.java:395 FileFunction.isPathPredicate
fn is_path_predicate(path: &Path) -> impl Fn(&RootedPath) -> bool + '_ {
    move |rooted_path: &RootedPath| rooted_path.as_path() == *path
}
