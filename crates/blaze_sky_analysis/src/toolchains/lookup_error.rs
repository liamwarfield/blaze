//! Error plumbing shared by the toolchain lookup helpers.

use blaze_analysis::toolchains::ToolchainException;
use blaze_packages::exceptions::{
    NoSuchPackageException, NoSuchTargetException, NoSuchThingException,
};
use blaze_skyframe::{ComputeError, Interrupted, SkyError, SkyFunctionException, Transience};

use crate::producers::errors::ConfiguredValueCreationException;

/// What the Java lookup helpers throw: `InterruptedException` or a `ToolchainException`
/// subclass.
#[derive(Clone, Debug)]
pub enum ToolchainLookupError {
    Interrupted(Interrupted),
    Toolchain(ToolchainException),
}

impl From<Interrupted> for ToolchainLookupError {
    fn from(e: Interrupted) -> Self {
        ToolchainLookupError::Interrupted(e)
    }
}

impl From<ToolchainException> for ToolchainLookupError {
    fn from(e: ToolchainException) -> Self {
        ToolchainLookupError::Toolchain(e)
    }
}

impl ToolchainLookupError {
    /// Rethrows from a SkyFunction: a `ToolchainException` becomes a persistent
    /// `SkyFunctionException`.
    pub fn into_compute_error(self) -> ComputeError {
        match self {
            ToolchainLookupError::Interrupted(i) => ComputeError::Interrupted(i),
            ToolchainLookupError::Toolchain(e) => {
                SkyFunctionException::new(e, Transience::Persistent).into()
            }
        }
    }
}

/// The exceptions a `CONFIGURED_TARGET` dependency can throw that the toolchain helpers catch
/// (Java `getOrThrow(key, ConfiguredValueCreationException.class, NoSuchThingException.class,
/// ActionConflictException.class)`).
///
/// PORT: `ActionConflictException` has no Rust error type yet (configured target creation
/// reports action conflicts as `CreateConfiguredTargetError::ActionConflict`); it is not caught.
#[derive(Clone, Debug)]
pub enum ConfiguredTargetLookupError {
    /// A `ConfiguredValueCreationException` (kept as the shared error).
    ConfiguredValueCreation(SkyError),
    NoSuchThing(NoSuchThingException),
}

/// Classifies a `CONFIGURED_TARGET` dependency error.
pub fn catch_configured_target_error(e: &SkyError) -> Option<ConfiguredTargetLookupError> {
    if e.downcast_ref::<ConfiguredValueCreationException>()
        .is_some()
    {
        return Some(ConfiguredTargetLookupError::ConfiguredValueCreation(
            e.clone(),
        ));
    }
    if let Some(x) = e.downcast_ref::<NoSuchThingException>() {
        return Some(ConfiguredTargetLookupError::NoSuchThing(x.clone()));
    }
    if let Some(x) = e.downcast_ref::<NoSuchPackageException>() {
        return Some(ConfiguredTargetLookupError::NoSuchThing(
            NoSuchThingException::Package(x.clone()),
        ));
    }
    if let Some(x) = e.downcast_ref::<NoSuchTargetException>() {
        return Some(ConfiguredTargetLookupError::NoSuchThing(
            NoSuchThingException::Target(x.clone()),
        ));
    }
    None
}

/// Catches only `ConfiguredValueCreationException` (Java
/// `getOrThrow(key, ConfiguredValueCreationException.class)`).
pub fn catch_configured_value_creation_exception(e: &SkyError) -> Option<SkyError> {
    if e.downcast_ref::<ConfiguredValueCreationException>()
        .is_some()
    {
        Some(e.clone())
    } else {
        None
    }
}

/// Catches a dependency's `ToolchainException` of one of the given Java subclasses (Java
/// `getOrThrow(key, E1.class, E2.class)` with `ToolchainException` subclasses).
pub fn catch_toolchain_exception(
    kinds: &'static [blaze_analysis::toolchains::ToolchainExceptionKind],
) -> impl Fn(&SkyError) -> Option<ToolchainException> {
    move |e: &SkyError| {
        e.downcast_ref::<ToolchainException>()
            .filter(|t| kinds.contains(&t.kind()))
            .cloned()
    }
}
