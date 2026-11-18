//! Port of `packages/License.java`.

use std::collections::BTreeSet;
use std::fmt;

use blaze_cmdline::Label;

/// Java `License.LicenseParsingException`.
// Java: com/google/devtools/build/lib/packages/License.java:47 License.LicenseParsingException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct LicenseParsingException(pub String);

/// Java `License.LicenseType`. Declared from most to least restrictive; `Ord` is Java's enum
/// order.
// Java: com/google/devtools/build/lib/packages/License.java:60 License.LicenseType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LicenseType {
    ByExceptionOnly,
    Restricted,
    RestrictedIfStaticallyLinked,
    Reciprocal,
    Notice,
    Permissive,
    Unencumbered,
    None,
}

// Java: com/google/devtools/build/lib/packages/License.java:42 License.EXCEPTION_PREFIX
const EXCEPTION_PREFIX: &str = "exception=";

impl LicenseType {
    /// The Java enum constant name (`LicenseType.name()` / `toString()`).
    pub fn name(self) -> &'static str {
        match self {
            LicenseType::ByExceptionOnly => "BY_EXCEPTION_ONLY",
            LicenseType::Restricted => "RESTRICTED",
            LicenseType::RestrictedIfStaticallyLinked => "RESTRICTED_IF_STATICALLY_LINKED",
            LicenseType::Reciprocal => "RECIPROCAL",
            LicenseType::Notice => "NOTICE",
            LicenseType::Permissive => "PERMISSIVE",
            LicenseType::Unencumbered => "UNENCUMBERED",
            LicenseType::None => "NONE",
        }
    }

    /// Java `LicenseType.valueOf(name)`.
    pub fn value_of(name: &str) -> Option<LicenseType> {
        Some(match name {
            "BY_EXCEPTION_ONLY" => LicenseType::ByExceptionOnly,
            "RESTRICTED" => LicenseType::Restricted,
            "RESTRICTED_IF_STATICALLY_LINKED" => LicenseType::RestrictedIfStaticallyLinked,
            "RECIPROCAL" => LicenseType::Reciprocal,
            "NOTICE" => LicenseType::Notice,
            "PERMISSIVE" => LicenseType::Permissive,
            "UNENCUMBERED" => LicenseType::Unencumbered,
            "NONE" => LicenseType::None,
            _ => return None,
        })
    }
}

/// The license of a target: license types plus exception labels (Java `License`).
// Java: com/google/devtools/build/lib/packages/License.java:39 License
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct License {
    // PORT: Java ImmutableSet built from an EnumSet: iteration in enum order, so a BTreeSet.
    license_types: BTreeSet<LicenseType>,
    exceptions: Vec<Label>,
}

impl License {
    // Java: com/google/devtools/build/lib/packages/License.java:79 License.leastRestrictive
    pub fn least_restrictive(types: &[LicenseType]) -> LicenseType {
        types
            .iter()
            .copied()
            .max()
            .unwrap_or(LicenseType::ByExceptionOnly)
    }

    /// Java `License.NO_LICENSE`.
    // Java: com/google/devtools/build/lib/packages/License.java:88 License.NO_LICENSE
    pub fn no_license() -> License {
        License {
            license_types: BTreeSet::from([LicenseType::None]),
            exceptions: Vec::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/License.java:97 License.of
    pub fn of(licenses: &[LicenseType], exceptions: &[Label]) -> License {
        // PORT: Java keeps the given iteration order of the license types; every caller passes
        // an EnumSet, whose order is the enum order kept by the BTreeSet.
        let license_set: BTreeSet<LicenseType> = licenses.iter().copied().collect();
        let mut exception_set: Vec<Label> = Vec::new();
        for e in exceptions {
            if !exception_set.contains(e) {
                exception_set.push(e.clone());
            }
        }
        if exception_set.is_empty() && license_set == BTreeSet::from([LicenseType::None]) {
            return License::no_license();
        }
        License {
            license_types: license_set,
            exceptions: exception_set,
        }
    }

    // Java: com/google/devtools/build/lib/packages/License.java:115 License.parseLicense
    pub fn parse_license(lic_strings: &[String]) -> Result<License, LicenseParsingException> {
        // The semantics of comparison for licenses depends on a stable iteration order for both
        // license types and exceptions. For licenseTypes, it will be the comparison order from
        // the enumerated types; for exceptions, it will be lexicographic order achieved using
        // TreeSets.
        let mut license_types: BTreeSet<LicenseType> = BTreeSet::new();
        let mut exceptions: BTreeSet<Label> = BTreeSet::new();
        for str in lic_strings {
            if let Some(rest) = str.strip_prefix(EXCEPTION_PREFIX) {
                let label = Label::parse_canonical(rest)
                    .map_err(|e| LicenseParsingException(e.get_message().to_owned()))?;
                exceptions.insert(label);
            } else {
                match LicenseType::value_of(&str.to_ascii_uppercase()) {
                    Some(t) => {
                        license_types.insert(t);
                    }
                    None => {
                        return Err(LicenseParsingException(format!(
                            "invalid license type: '{str}'"
                        )));
                    }
                }
            }
        }
        let license_types: Vec<LicenseType> = license_types.into_iter().collect();
        let exceptions: Vec<Label> = exceptions.into_iter().collect();
        Ok(License::of(&license_types, &exceptions))
    }

    // Java: com/google/devtools/build/lib/packages/License.java:148 License.getLicenseTypes
    pub fn get_license_types(&self) -> &BTreeSet<LicenseType> {
        &self.license_types
    }

    // Java: com/google/devtools/build/lib/packages/License.java:156 License.getExceptions
    pub fn get_exceptions(&self) -> &[Label] {
        &self.exceptions
    }

    // Java: com/google/devtools/build/lib/packages/License.java:161 License.isSpecified
    pub fn is_specified(&self) -> bool {
        *self != License::no_license()
    }
}

impl License {
    /// The Starlark repr: a canonically ordered list of strings that `parse_license` parses back
    /// to an equal license.
    // Java: com/google/devtools/build/lib/packages/License.java:210 License.repr
    pub fn repr(&self) -> String {
        let items: Vec<String> = self
            .license_types
            .iter()
            .map(|t| format!("\"{}\"", t.name().to_ascii_lowercase()))
            .chain(
                self.exceptions
                    .iter()
                    .map(|l| format!("\"{EXCEPTION_PREFIX}{}\"", l.get_canonical_form())),
            )
            .collect();
        format!("[{}]", items.join(", "))
    }
}

impl fmt::Display for License {
    /// A canonical form of the license (enum order of types, lexicographic exceptions).
    // Java: com/google/devtools/build/lib/packages/License.java:172 License.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let types: Vec<&str> = self.license_types.iter().map(|t| t.name()).collect();
        let types = format!("[{}]", types.join(", ")).to_ascii_lowercase();
        if self.exceptions.is_empty() {
            f.write_str(&types)
        } else {
            let exceptions: Vec<String> = self.exceptions.iter().map(|l| l.to_string()).collect();
            write!(f, "{types} with exceptions [{}]", exceptions.join(", "))
        }
    }
}
