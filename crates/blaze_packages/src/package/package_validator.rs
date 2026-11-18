//! Port of `packages/PackageValidator.java`.

use blaze_events::ExtendedEventHandler;

use crate::exceptions::NoSuchPackageException;
use crate::package::Package;
use crate::package::PackageLimits;

/// Java `PackageValidator.InvalidPackageException` (a `NoSuchPackageException` with the
/// validator's message).
// Java: com/google/devtools/build/lib/packages/PackageValidator.java:28 PackageValidator.InvalidPackageException
pub type InvalidPackageException = NoSuchPackageException;

/// Validates a loaded package (Java `PackageValidator`).
// PORT: the `Metrics` parameter (PackageLoadingListener.Metrics) is omitted (profiling only).
// Java: com/google/devtools/build/lib/packages/PackageValidator.java:26 PackageValidator
pub trait PackageValidator: Send + Sync {
    // Java: com/google/devtools/build/lib/packages/PackageValidator.java:52 PackageValidator.getPackageLimits
    fn get_package_limits(&self) -> PackageLimits {
        PackageLimits::default()
    }

    // Java: com/google/devtools/build/lib/packages/PackageValidator.java:59 PackageValidator.validate
    fn validate(
        &self,
        pkg: &Package,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<(), InvalidPackageException>;
}

/// Java `PackageValidator.NOOP_VALIDATOR`.
// Java: com/google/devtools/build/lib/packages/PackageValidator.java:30 PackageValidator.NOOP_VALIDATOR
pub struct NoopValidator;

impl PackageValidator for NoopValidator {
    fn validate(
        &self,
        _pkg: &Package,
        _event_handler: &dyn ExtendedEventHandler,
    ) -> Result<(), InvalidPackageException> {
        Ok(())
    }
}
