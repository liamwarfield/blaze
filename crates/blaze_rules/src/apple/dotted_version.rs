//! The Starlark face of `rules/apple/DottedVersion.java` (`starlarkbuildapi/apple/DottedVersionApi.java`,
//! type `DottedVersion`) over `blaze_optdefs::fragment::dotted_version::DottedVersion`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_optdefs::fragment::dotted_version::DottedVersion;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::environment::{Methods, MethodsBuilder, MethodsStatic};
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{Demand, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value};

/// A `DottedVersion` as a Starlark value.
// Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:80 DottedVersion
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkDottedVersion(#[allocative(skip)] pub DottedVersion);

starlark_simple_value!(StarlarkDottedVersion);

impl StarlarkDottedVersion {
    /// `x instanceof DottedVersion ? (DottedVersion) x : null`.
    pub fn version_from_value<'v>(v: Value<'v>) -> Option<&'v DottedVersion> {
        v.downcast_ref::<StarlarkDottedVersion>().map(|d| &d.0)
    }
}

impl fmt::Display for StarlarkDottedVersion {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:394 DottedVersion.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// `Integer` result of `compareTo` (Guava `ComparisonChain.result()` is -1, 0 or 1).
fn ordering_to_int(o: Ordering) -> i32 {
    match o {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[starlark_value(type = "DottedVersion")]
impl<'v> StarlarkValue<'v> for StarlarkDottedVersion {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("DottedVersion", dotted_version_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:394 DottedVersion.repr
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&self.0.to_string());
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:370 DottedVersion.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(StarlarkDottedVersion::version_from_value(other)
            .is_some_and(|o| self.0.compare_to(o) == Ordering::Equal))
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:382 DottedVersion.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.hash(hasher);
        Ok(())
    }

    // Java: net/starlark/java/eval/EvalUtils.java EvalUtils.compare (Comparable values of the
    // same class compare with compareTo).
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:278 DottedVersion.compareTo
    fn compare(&self, other: Value<'v>) -> starlark::Result<Ordering> {
        match StarlarkDottedVersion::version_from_value(other) {
            Some(o) => Ok(self.0.compare_to(o)),
            None => Err(starlark::Error::new_other(anyhow::anyhow!(
                "unsupported comparison: DottedVersion <=> {}",
                other.get_type()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:273 DottedVersion.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

#[starlark_module]
fn dotted_version_methods(builder: &mut MethodsBuilder) {
    /// Compares based on most significant (first) not-matching version component. So, for
    /// example, 1.2.3 < 1.2.4
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:292 DottedVersion.compareTo_starlark
    fn compare_to<'v>(
        this: &StarlarkDottedVersion,
        #[starlark(require = pos)] other: Value<'v>,
    ) -> starlark::Result<i32> {
        match StarlarkDottedVersion::version_from_value(other) {
            Some(o) => Ok(ordering_to_int(this.0.compare_to(o))),
            None => Err(starlark::Error::new_other(anyhow::anyhow!(
                "in call to compare_to(), parameter 'other' got value of type '{}', want \
                 'DottedVersion'",
                other.get_type()
            ))),
        }
    }

    /// Returns the string representation of a dotted version.
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:365 DottedVersion.toString
    #[starlark(attribute)]
    fn to_string(this: &StarlarkDottedVersion) -> starlark::Result<String> {
        Ok(this.0.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> StarlarkDottedVersion {
        StarlarkDottedVersion(DottedVersion::from_string(s).unwrap())
    }

    // Java: DottedVersionTest.testCompareTo (the Starlark compare_to view of it)
    #[test]
    fn compare_to() {
        assert_eq!(ordering_to_int(v("1.2.3").0.compare_to(&v("1.2.4").0)), -1);
        assert_eq!(ordering_to_int(v("1.2").0.compare_to(&v("1.2.0").0)), 0);
        assert_eq!(ordering_to_int(v("5.0.1").0.compare_to(&v("5.0").0)), 1);
    }

    #[test]
    fn repr_is_original_string() {
        assert_eq!(v("10.0.0").to_string(), "10.0.0");
        assert_eq!(v("8.4").to_string(), "8.4");
    }
}
