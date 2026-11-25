//! FILE_SYMLINK_CYCLE_UNIQUENESS and FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS: functions whose
//! only purpose is the side effect of reporting each unique symlink error exactly once.
//!
//! Java: com/google/devtools/build/lib/io/{AbstractFileChainUniquenessFunction,
//! FileSymlinkCycleUniquenessFunction, FileSymlinkInfiniteExpansionUniquenessFunction}.java.
//! PORT: move to blaze_vfs (Java `lib/io`) once blaze_vfs registers SkyFunctions.

use std::fmt;

use blaze_events::Event;
use blaze_skyframe::{ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionName};
use blaze_vfs::RootedPath;

use crate::sky_functions;

/// The value of the uniqueness functions. Java: `EmptySkyValue.INSTANCE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptySkyValue;

/// Creates a canonicalized representation of the cycle specified by `cycle`. `cycle` must be
/// non-empty. The representation may not be unique if cycle has duplicate elements.
// Java: com/google/devtools/build/lib/io/AbstractFileChainUniquenessFunction.java:65 AbstractFileChainUniquenessFunction.canonicalize
pub fn canonicalize(cycle: &[RootedPath]) -> Vec<RootedPath> {
    let mut min_pos = 0;
    let mut min = &cycle[0];
    for (i, cur) in cycle.iter().enumerate().skip(1) {
        if cur < min {
            min_pos = i;
            min = cur;
        }
    }
    let mut out = Vec::with_capacity(cycle.len());
    out.extend_from_slice(&cycle[min_pos..]);
    out.extend_from_slice(&cycle[..min_pos]);
    out
}

fn fmt_chain(
    f: &mut fmt::Formatter<'_>,
    name: SkyFunctionName,
    chain: &[RootedPath],
) -> fmt::Result {
    // Java AbstractSkyKey.toString: functionName() + ":" + arg (an ImmutableList).
    write!(f, "{}:[", name)?;
    for (i, p) in chain.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{}", p)?;
    }
    f.write_str("]")
}

/// Key of FILE_SYMLINK_CYCLE_UNIQUENESS: a canonicalized cycle.
// Java: com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:39 FileSymlinkCycleUniquenessFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileSymlinkCycleUniquenessKey(Vec<RootedPath>);

impl FileSymlinkCycleUniquenessKey {
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:34 FileSymlinkCycleUniquenessFunction.key
    pub fn key(cycle: &[RootedPath]) -> Self {
        FileSymlinkCycleUniquenessKey(canonicalize(cycle))
    }
}

impl fmt::Debug for FileSymlinkCycleUniquenessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_chain(f, Self::FUNCTION, &self.0)
    }
}

impl Key for FileSymlinkCycleUniquenessKey {
    type Value = EmptySkyValue;
    const FUNCTION: SkyFunctionName = sky_functions::FILE_SYMLINK_CYCLE_UNIQUENESS;
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:32 FileSymlinkCycleUniquenessFunction.NAME (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns the singleton `EmptySkyValue.INSTANCE`, equal to itself.
    // Java: com/google/devtools/build/skyframe/EmptySkyValue.java:24 EmptySkyValue.INSTANCE
    fn values_equal(old: &EmptySkyValue, new: &EmptySkyValue) -> bool {
        old == new
    }
}

/// Key of FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS: a canonicalized chain.
// Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:40 FileSymlinkInfiniteExpansionUniquenessFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct FileSymlinkInfiniteExpansionUniquenessKey(Vec<RootedPath>);

impl FileSymlinkInfiniteExpansionUniquenessKey {
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:35 FileSymlinkInfiniteExpansionUniquenessFunction.key
    pub fn key(cycle: &[RootedPath]) -> Self {
        FileSymlinkInfiniteExpansionUniquenessKey(canonicalize(cycle))
    }
}

impl fmt::Debug for FileSymlinkInfiniteExpansionUniquenessKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_chain(f, Self::FUNCTION, &self.0)
    }
}

impl Key for FileSymlinkInfiniteExpansionUniquenessKey {
    type Value = EmptySkyValue;
    const FUNCTION: SkyFunctionName = sky_functions::FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS;
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:33 FileSymlinkInfiniteExpansionUniquenessFunction.NAME (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the function returns the singleton `EmptySkyValue.INSTANCE`, equal to itself.
    // Java: com/google/devtools/build/skyframe/EmptySkyValue.java:24 EmptySkyValue.INSTANCE
    fn values_equal(old: &EmptySkyValue, new: &EmptySkyValue) -> bool {
        old == new
    }
}

/// Given a "cycle" of [`RootedPath`] files, emits an error message for this cycle.
// Java: com/google/devtools/build/lib/io/AbstractFileChainUniquenessFunction.java:44 AbstractFileChainUniquenessFunction.compute
fn compute_chain(
    env: &mut Environment<'_>,
    chain: &[RootedPath],
    concise_description: &str,
    header_message: &str,
    footer_message: &str,
) -> Result<Option<EmptySkyValue>, ComputeError> {
    let mut error_message = String::new();
    error_message.push_str(&format!("{} detected\n", concise_description));
    error_message.push_str(&format!("{}\n", header_message));
    for elt in chain {
        // Java: elementToString(elt) = elt.asPath().toString()
        error_message.push_str(&format!("{}\n", elt.as_path()));
    }
    error_message.push_str(&format!("{}\n", footer_message));
    // The purpose of this SkyFunction is the side effect of emitting an error message exactly
    // once per build per unique error.
    env.listener().handle(&Event::error(error_message));
    Ok(Some(EmptySkyValue))
}

/// A SkyFunction that has the side effect of reporting a file symlink cycle.
// Java: com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:30 FileSymlinkCycleUniquenessFunction
pub fn file_symlink_cycle_uniqueness_function(
    key: &FileSymlinkCycleUniquenessKey,
    env: &mut Environment<'_>,
) -> Result<Option<EmptySkyValue>, ComputeError> {
    compute_chain(
        env,
        &key.0,
        "circular symlinks",
        "[start of symlink cycle]",
        "[end of symlink cycle]",
    )
}

/// A SkyFunction that has the side effect of reporting a file symlink expansion error exactly
/// once.
// Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:30 FileSymlinkInfiniteExpansionUniquenessFunction
pub fn file_symlink_infinite_expansion_uniqueness_function(
    key: &FileSymlinkInfiniteExpansionUniquenessKey,
    env: &mut Environment<'_>,
) -> Result<Option<EmptySkyValue>, ComputeError> {
    compute_chain(
        env,
        &key.0,
        "infinite symlink expansion",
        "[start of symlink chain]",
        "[end of symlink chain]",
    )
}
