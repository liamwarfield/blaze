//! Port of `skyframe/toolchains/ToolchainException.java` and its subclasses.
//!
//! PORT: Java's abstract `ToolchainException` has one subclass per failure (in several files and
//! packages). The Rust port is one error type carrying the subclass as a
//! [`ToolchainExceptionKind`], so every crate can create and match it.

use std::error::Error;
use std::fmt;

use blaze_packages::exceptions::{NoSuchPackageKind, NoSuchThingException};
use blaze_skyframe::SkyError;

/// The Java subclass of a [`ToolchainException`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolchainExceptionKind {
    /// Java `PlatformLookupUtil.InvalidPlatformException`.
    InvalidPlatform,
    /// Java `ConstraintValueLookupUtil.InvalidConstraintValueException`.
    InvalidConstraintValue,
    /// Java `ToolchainTypeLookupUtil.InvalidToolchainTypeException`.
    InvalidToolchainType,
    /// Java `RegisteredToolchainsFunction.InvalidToolchainLabelException`.
    InvalidToolchainLabel,
    /// Java `RegisteredExecutionPlatformsFunction.InvalidExecutionPlatformLabelException`.
    InvalidExecutionPlatformLabel,
    /// Java `NoMatchingPlatformException`.
    NoMatchingPlatform,
    /// Java `ToolchainResolutionFunction.UnresolvedToolchainsException`.
    UnresolvedToolchains,
    /// Java `SingleToolchainResolutionFunction.InvalidConfigurationDuringToolchainResolutionException`.
    InvalidConfigurationDuringToolchainResolution,
    /// Java `ResolvedToolchainContext.TargetNotToolchainException`.
    TargetNotToolchain,
    /// Java `ResolvedToolchainContext.MissingToolchainTypeRequirementException`.
    MissingToolchainTypeRequirement,
}

impl ToolchainExceptionKind {
    /// Java `getDetailedCode()`: the `FailureDetails.Toolchain.Code` name.
    pub fn get_detailed_code(self) -> &'static str {
        match self {
            // Java: com/google/devtools/build/lib/skyframe/toolchains/PlatformLookupUtil.java:197 PlatformLookupUtil.InvalidPlatformException.getDetailedCode
            ToolchainExceptionKind::InvalidPlatform => "INVALID_PLATFORM_VALUE",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/ConstraintValueLookupUtil.java:114 ConstraintValueLookupUtil.InvalidConstraintValueException.getDetailedCode
            ToolchainExceptionKind::InvalidConstraintValue => "INVALID_CONSTRAINT_VALUE",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainTypeLookupUtil.java:170 ToolchainTypeLookupUtil.InvalidToolchainTypeException.getDetailedCode
            ToolchainExceptionKind::InvalidToolchainType => "INVALID_TOOLCHAIN_TYPE",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredToolchainsFunction.java:255 RegisteredToolchainsFunction.InvalidToolchainLabelException.getDetailedCode
            ToolchainExceptionKind::InvalidToolchainLabel => "INVALID_TOOLCHAIN",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/RegisteredExecutionPlatformsFunction.java:271 RegisteredExecutionPlatformsFunction.InvalidExecutionPlatformLabelException.getDetailedCode
            ToolchainExceptionKind::InvalidExecutionPlatformLabel => "INVALID_PLATFORM_VALUE",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/NoMatchingPlatformException.java:25 NoMatchingPlatformException.getDetailedCode
            ToolchainExceptionKind::NoMatchingPlatform => "NO_MATCHING_EXECUTION_PLATFORM",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainResolutionFunction.java:389 ToolchainResolutionFunction.UnresolvedToolchainsException.getDetailedCode
            ToolchainExceptionKind::UnresolvedToolchains => "NO_MATCHING_TOOLCHAIN",
            // Java: com/google/devtools/build/lib/skyframe/toolchains/SingleToolchainResolutionFunction.java:297 SingleToolchainResolutionFunction.InvalidConfigurationDuringToolchainResolutionException.getDetailedCode
            ToolchainExceptionKind::InvalidConfigurationDuringToolchainResolution => {
                "INVALID_CONSTRAINT_VALUE"
            }
            // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:163 ResolvedToolchainContext.TargetNotToolchainException.getDetailedCode
            ToolchainExceptionKind::TargetNotToolchain => "MISSING_PROVIDER",
            // Java: com/google/devtools/build/lib/analysis/ResolvedToolchainContext.java:179 ResolvedToolchainContext.MissingToolchainTypeRequirementException.getDetailedCode
            ToolchainExceptionKind::MissingToolchainTypeRequirement => "NO_MATCHING_TOOLCHAIN",
        }
    }
}

/// Base class for exceptions that happen during toolchain resolution.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:34 ToolchainException
#[derive(Clone)]
pub struct ToolchainException {
    kind: ToolchainExceptionKind,
    /// Java `getMessage()`.
    message: String,
    /// Java `getCause()`.
    cause: Option<SkyError>,
}

impl ToolchainException {
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:36 ToolchainException.<init>
    pub fn new(kind: ToolchainExceptionKind, message: impl Into<String>) -> Self {
        ToolchainException {
            kind,
            message: message.into(),
            cause: None,
        }
    }

    /// Java `ToolchainException(Throwable cause)`: the message is `cause.toString()`, i.e. the
    /// cause's Java class name, `": "`, and its message.
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:40 ToolchainException.<init>
    pub fn from_cause(
        kind: ToolchainExceptionKind,
        cause_java_class_name: &str,
        cause: SkyError,
    ) -> Self {
        ToolchainException {
            kind,
            message: format!("{cause_java_class_name}: {cause}"),
            cause: Some(cause),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:44 ToolchainException.<init>
    pub fn with_cause(
        kind: ToolchainExceptionKind,
        message: impl Into<String>,
        cause: SkyError,
    ) -> Self {
        ToolchainException {
            kind,
            message: message.into(),
            cause: Some(cause),
        }
    }

    /// The Java subclass.
    pub fn kind(&self) -> ToolchainExceptionKind {
        self.kind
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_cause(&self) -> Option<&SkyError> {
        self.cause.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:48 ToolchainException.getDetailedCode
    pub fn get_detailed_code(&self) -> &'static str {
        self.kind.get_detailed_code()
    }

    /// Finds an exception of type `E` among the causes (Java's loop of
    /// `asConfiguredValueCreationException`).
    // Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainException.java:69 ToolchainException.asConfiguredValueCreationException
    // PORT: the rest of asConfiguredValueCreationException (creating a new
    // ConfiguredValueCreationException) belongs with that exception (agent C).
    pub fn find_cause<E: Error + 'static>(&self) -> Option<&E> {
        let mut cause: Option<&(dyn Error + 'static)> =
            self.cause.as_deref().map(|c| c as &(dyn Error + 'static));
        while let Some(c) = cause {
            if let Some(e) = c.downcast_ref::<E>() {
                return Some(e);
            }
            cause = c.source();
        }
        None
    }
}

impl fmt::Debug for ToolchainException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolchainException")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish()
    }
}

impl fmt::Display for ToolchainException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ToolchainException {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.cause.as_deref().map(|c| c as &(dyn Error + 'static))
    }
}

/// The Java class name of a `NoSuchThingException` (for `Throwable.toString`).
pub fn no_such_thing_java_class_name(e: &NoSuchThingException) -> &'static str {
    match e {
        NoSuchThingException::Package(p) => match p.kind() {
            NoSuchPackageKind::NoSuchPackage => {
                "com.google.devtools.build.lib.packages.NoSuchPackageException"
            }
            NoSuchPackageKind::BuildFileNotFound => {
                "com.google.devtools.build.lib.packages.BuildFileNotFoundException"
            }
            NoSuchPackageKind::BuildFileContainsErrors => {
                "com.google.devtools.build.lib.packages.BuildFileContainsErrorsException"
            }
            NoSuchPackageKind::InvalidPackageName => {
                "com.google.devtools.build.lib.packages.InvalidPackageNameException"
            }
            NoSuchPackageKind::RepositoryFetch => {
                "com.google.devtools.build.lib.packages.RepositoryFetchException"
            }
        },
        NoSuchThingException::Target(_) => {
            "com.google.devtools.build.lib.packages.NoSuchTargetException"
        }
        NoSuchThingException::ErrorDeterminingRepository(_) => {
            "com.google.devtools.build.lib.packages.ErrorDeterminingRepositoryException"
        }
        NoSuchThingException::Other { .. } => {
            "com.google.devtools.build.lib.packages.NoSuchThingException"
        }
    }
}

/// Java class name of `ConfiguredValueCreationException`.
pub const CONFIGURED_VALUE_CREATION_EXCEPTION_CLASS: &str =
    "com.google.devtools.build.lib.skyframe.ConfiguredValueCreationException";

/// Java class name of `InvalidConfigurationException`.
pub const INVALID_CONFIGURATION_EXCEPTION_CLASS: &str =
    "com.google.devtools.build.lib.analysis.config.InvalidConfigurationException";
