//! Error plumbing shared by the toolchain lookup helpers.

use std::sync::Arc;

use blaze_analysis::toolchains::ToolchainException;
use blaze_packages::exceptions::{
    NoSuchPackageException, NoSuchTargetException, NoSuchThingException,
};
use blaze_skyframe::{ComputeError, Interrupted, SkyError, SkyFunctionException, Transience};

use crate::producers::errors::{
    ConfiguredTargetFunctionException, ConfiguredValueCreationException,
};

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
#[derive(Clone, Debug)]
pub enum ConfiguredTargetLookupError {
    /// A `ConfiguredValueCreationException` (as a shared error, to be the cause of the
    /// `ToolchainException` the helper throws).
    ConfiguredValueCreation(SkyError),
    NoSuchThing(NoSuchThingException),
    /// An `ActionConflictException`.
    ///
    /// PORT: the exception has no Rust type of its own; `ConfiguredTargetFunction` throws it as
    /// `ConfiguredTargetFunctionException::ActionConflict(message)`, and that shared error is
    /// kept here as the cause.
    ActionConflict(SkyError),
}

/// Classifies a `CONFIGURED_TARGET` dependency error.
///
/// `ConfiguredTargetFunction` throws `ReportedException` / `UnreportedException` /
/// `DependencyException`, whose causes are the exception classes the Java callers name. Here
/// the cause is one `ConfiguredTargetFunctionException` (see `producers::errors`), so the wrapped
/// exception is taken out of it.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetEvaluationExceptions.java:28 ConfiguredTargetEvaluationExceptions.ReportedException
pub fn catch_configured_target_error(e: &SkyError) -> Option<ConfiguredTargetLookupError> {
    if let Some(x) = e.downcast_ref::<ConfiguredTargetFunctionException>() {
        if let Some(c) = x.as_configured_value_creation() {
            return Some(ConfiguredTargetLookupError::ConfiguredValueCreation(
                Arc::new(c.clone()),
            ));
        }
        return match x {
            ConfiguredTargetFunctionException::NoSuchThing(n) => {
                Some(ConfiguredTargetLookupError::NoSuchThing(n.clone()))
            }
            ConfiguredTargetFunctionException::ActionConflict(..) => {
                Some(ConfiguredTargetLookupError::ActionConflict(e.clone()))
            }
            // InconsistentNullConfigException is not one of the classes asked for: the
            // dependency is missing.
            _ => None,
        };
    }
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
/// `getOrThrow(key, ConfiguredValueCreationException.class)`). The returned shared error is the
/// `ConfiguredValueCreationException` itself (see [`catch_configured_target_error`]).
pub fn catch_configured_value_creation_exception(e: &SkyError) -> Option<SkyError> {
    if let Some(x) = e.downcast_ref::<ConfiguredTargetFunctionException>() {
        return x
            .as_configured_value_creation()
            .map(|c| Arc::new(c.clone()) as SkyError);
    }
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
