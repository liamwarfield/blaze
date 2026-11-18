//! Port of `rules/apple/DottedVersion.java` and `rules/apple/DottedVersionConverter.java`.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type,
};

use super::converters::ConvResult;

// Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:80 DottedVersion
/// Represents Xcode versions and allows parsing them.
///
/// Xcode versions are formed of multiple components, separated by periods, for example `4.5.6`
/// or `5.0.1beta2`. Components must start with a non-negative integer and at least one component
/// must be present.
///
/// Specifically, the format of a component is `\d+([a-z0-9]*?)?(\d+)?`.
///
/// Dotted versions are ordered using natural integer sorting on components in order from first
/// to last where any missing element is considered to have the value 0 if they don't contain any
/// non-numeric characters.
///
/// If the component contains any alphabetic characters after the leading integer, it is
/// considered *smaller* than any components with the same integer but larger than any component
/// with a smaller integer. If the integers are the same, the alphabetic sequences are compared
/// lexicographically, and if *they* turn out to be the same, the final (optional) integer is
/// compared. As with the leading integer, this final integer is considered to be 0 if not present.
#[derive(Clone, Debug)]
pub struct DottedVersion {
    components: Vec<Component>,
    string_representation: String,
    num_original_components: i32,
}

// Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:97 DottedVersion.Option
/// Wrapper class for [`DottedVersion`] whose equality is string equality.
///
/// This is necessary because Bazel assumes that `FragmentOptions` that are equal yield fragments
/// that are the same. However, this does not hold if the options hold a [`DottedVersion`] because
/// trailing zeroes are not considered significant when comparing them, but they do matter in
/// configuration fragments (for example, they end up in output directory names).
#[derive(Clone, Debug)]
pub struct DottedVersionOption {
    version: DottedVersion,
}

impl DottedVersionOption {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:104 DottedVersion.Option.get
    pub fn get(&self) -> &DottedVersion {
        &self.version
    }
}

impl std::fmt::Display for DottedVersionOption {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:119 DottedVersion.Option.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.version.string_representation)
    }
}

impl PartialEq for DottedVersionOption {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:129 DottedVersion.Option.equals
    fn eq(&self, other: &Self) -> bool {
        self.version.string_representation == other.version.string_representation
    }
}

impl Eq for DottedVersionOption {}

impl Hash for DottedVersionOption {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:124 DottedVersion.Option.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version.string_representation.hash(state);
    }
}

impl OptionScalar for DottedVersionOption {
    fn java_to_string(&self) -> String {
        self.to_string()
    }
}

option_field_type!(
    DottedVersionOption,
    "com.google.devtools.build.lib.rules.apple.DottedVersion$Option",
    raw = DottedVersionOption {
        version: DottedVersion::from_string_unchecked("0"),
    }
);

// Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:163 DottedVersion.InvalidDottedVersionException
/// Exception thrown when parsing an invalid dotted version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidDottedVersionException {
    message: String,
}

impl InvalidDottedVersionException {
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for InvalidDottedVersionException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `String.format(ILLEGAL_VERSION, version)`.
fn illegal_version(version: &str) -> InvalidDottedVersionException {
    InvalidDottedVersionException {
        message: format!(
            "Dotted version components must all start with the form \\d+([a-z0-9]*?)?(\\d+)? but \
             got '{}'",
            version
        ),
    }
}

impl DottedVersion {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:143 DottedVersion.maybeUnwrap
    pub fn maybe_unwrap(option: Option<&DottedVersionOption>) -> Option<DottedVersion> {
        option.map(|o| o.get().clone())
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:148 DottedVersion.option
    pub fn option(version: Option<DottedVersion>) -> Option<DottedVersionOption> {
        version.map(|version| DottedVersionOption { version })
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:177 DottedVersion.fromStringUnchecked
    /// Create a dotted version by parsing the given version string. Throws an unchecked exception
    /// if the argument is malformed.
    pub fn from_string_unchecked(version: &str) -> DottedVersion {
        match Self::from_string(version) {
            Ok(v) => v,
            Err(e) => panic!("IllegalArgumentException: {}", e),
        }
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:190 DottedVersion.fromString
    /// Generates a new dotted version from the given version string.
    pub fn from_string(version: &str) -> Result<DottedVersion, InvalidDottedVersionException> {
        if version.is_empty() {
            return Err(illegal_version(version));
        }
        let mut components = Vec::new();
        for component in version.split('.') {
            if Self::is_descriptive_component(component) {
                break;
            }
            components.push(Self::to_component(component, version)?);
        }

        if components.is_empty() {
            return Err(illegal_version(version));
        }

        let num_original_components = components.len() as i32;

        // Remove trailing (but not the first or middle) zero components for easier comparison and
        // hashcoding.
        let mut i = components.len() - 1;
        while i > 0 {
            if components[i] == Component::zero() {
                components.remove(i);
            } else {
                break;
            }
            i -= 1;
        }

        Ok(DottedVersion {
            components,
            string_representation: version.to_string(),
            num_original_components,
        })
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:223 DottedVersion.isDescriptiveComponent
    // Some of special build versions contains descriptive components like "experimental" or
    // "internal". These components are usually by the end of version number, and can be ignored.
    /// `DESCRIPTIVE_COMPONENT_PATTERN = ([a-z]\w*)`, case-insensitive, `matches()`.
    fn is_descriptive_component(component: &str) -> bool {
        let mut chars = component.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:227 DottedVersion.toComponent
    /// `COMPONENT_PATTERN = (\d+)([a-z0-9]*?)?(\d+)?`, case-insensitive, `matches()`: the
    /// leading digit run, then the shortest alphanumeric run that leaves only digits (or
    /// nothing) behind.
    fn to_component(
        component: &str,
        version: &str,
    ) -> Result<Component, InvalidDottedVersionException> {
        let lead = component.bytes().take_while(u8::is_ascii_digit).count();
        let rest = &component[lead..];
        if lead == 0 || !rest.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(illegal_version(version));
        }
        let trailing = rest.bytes().rev().take_while(u8::is_ascii_digit).count();
        let alpha = &rest[..rest.len() - trailing];
        let second = &rest[rest.len() - trailing..];

        let first_number = Self::parse_number(&component[..lead], version)?;
        let alpha_sequence = if alpha.is_empty() {
            None
        } else {
            Some(alpha.to_string())
        };
        let second_number = if second.is_empty() {
            0
        } else {
            Self::parse_number(second, version)?
        };
        Ok(Component {
            first_number,
            alpha_sequence,
            second_number,
            string_representation: component.to_string(),
        })
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:250 DottedVersion.parseNumber
    fn parse_number(s: &str, version: &str) -> Result<i32, InvalidDottedVersionException> {
        s.parse::<i32>().map_err(|_| illegal_version(version))
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:278 DottedVersion.compareTo
    pub fn compare_to(&self, other: &DottedVersion) -> Ordering {
        let max_components = self.components.len().max(other.components.len());
        for component_index in 0..max_components {
            let my_component = self.get_component(component_index);
            let other_component = other.get_component(component_index);
            let comparison = my_component.compare_to(&other_component);
            if comparison != Ordering::Equal {
                return comparison;
            }
        }
        Ordering::Equal
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:306 DottedVersion.toStringWithComponents
    /// Returns the string representation of this dotted version, padded or truncated to the
    /// specified number of components.
    pub fn to_string_with_components(&self, num_components: i32) -> String {
        assert!(
            num_components > 0,
            "IllegalArgumentException: Can't serialize as a version with {} components",
            num_components
        );
        let n = num_components as usize;
        let mut string_components: Vec<String> = Vec::new();
        if n <= self.components.len() {
            string_components.extend(
                self.components[..n]
                    .iter()
                    .map(|c| c.string_representation.clone()),
            );
        } else {
            string_components.extend(
                self.components
                    .iter()
                    .map(|c| c.string_representation.clone()),
            );
            for _ in self.components.len()..n {
                string_components.push(Component::zero().string_representation);
            }
        }
        string_components.join(".")
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:335 DottedVersion.toStringWithMinimumComponents
    /// Returns the string representation of this dotted version, padded to a minimum number of
    /// components if the string representation does not already contain that many components.
    pub fn to_string_with_minimum_components(&self, num_min_components: i32) -> String {
        self.to_string_with_components(self.num_original_components.max(num_min_components))
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:343 DottedVersion.hasAlphabeticCharacters
    /// Returns true if this version number has any alphabetic characters, such as 'alpha' in
    /// "7.3alpha.2".
    pub fn has_alphabetic_characters(&self) -> bool {
        self.components.iter().any(|c| c.alpha_sequence.is_some())
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:356 DottedVersion.numComponents
    /// Returns the number of components in this version number.
    pub fn num_components(&self) -> i32 {
        self.components.len() as i32
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:386 DottedVersion.getComponent
    fn get_component(&self, group_index: usize) -> Component {
        self.components
            .get(group_index)
            .cloned()
            .unwrap_or_else(Component::zero)
    }
}

impl std::fmt::Display for DottedVersion {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:365 DottedVersion.toString
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.string_representation)
    }
}

impl PartialEq for DottedVersion {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:370 DottedVersion.equals
    fn eq(&self, other: &Self) -> bool {
        self.compare_to(other) == Ordering::Equal
    }
}

impl Eq for DottedVersion {}

impl PartialOrd for DottedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DottedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare_to(other)
    }
}

impl Hash for DottedVersion {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:382 DottedVersion.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        for c in &self.components {
            c.hash(state);
        }
    }
}

// Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:398 DottedVersion.Component
#[derive(Clone, Debug)]
struct Component {
    first_number: i32,
    alpha_sequence: Option<String>,
    second_number: i32,
    string_representation: String,
}

impl Component {
    /// `ZERO_COMPONENT`.
    fn zero() -> Component {
        Component {
            first_number: 0,
            alpha_sequence: None,
            second_number: 0,
            string_representation: "0".to_string(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/apple/DottedVersion.java:416 DottedVersion.Component.compareTo
    fn compare_to(&self, other: &Component) -> Ordering {
        self.first_number
            .cmp(&other.first_number)
            // Ordering.natural().nullsLast()
            .then_with(|| match (&self.alpha_sequence, &other.alpha_sequence) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => super::fragment_options::java_string_compare(a, b),
            })
            .then_with(|| self.second_number.cmp(&other.second_number))
    }
}

impl PartialEq for Component {
    fn eq(&self, other: &Self) -> bool {
        self.compare_to(other) == Ordering::Equal
    }
}

impl Hash for Component {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.first_number.hash(state);
        self.alpha_sequence.hash(state);
        self.second_number.hash(state);
    }
}

// Java: com/google/devtools/build/lib/rules/apple/DottedVersionConverter.java:21 DottedVersionConverter
/// Converter for options representing [`DottedVersion`] values.
#[derive(Clone, Copy, Debug, Default)]
pub struct DottedVersionConverter;

impl Converter for DottedVersionConverter {
    // Java: com/google/devtools/build/lib/rules/apple/DottedVersionConverter.java:24 DottedVersionConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        match DottedVersion::from_string(non_null(input)) {
            Ok(v) => Ok(DottedVersion::option(Some(v)).map(Value::new)),
            Err(e) => Err(OptionsParsingException::new(e.get_message())),
        }
    }
    fn get_type_description(&self) -> String {
        "a dotted version (for example '2.3' or '3.3alpha2.4')".into()
    }
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/rules/apple/DottedVersionTest.java
    use super::*;

    fn v(s: &str) -> DottedVersion {
        DottedVersion::from_string(s).unwrap()
    }

    fn check_groups(groups: &[&[&str]], ordered: bool) {
        for (i, gi) in groups.iter().enumerate() {
            for (j, gj) in groups.iter().enumerate() {
                for a in gi.iter() {
                    for b in gj.iter() {
                        assert_eq!(v(a) == v(b), i == j, "{} vs {}", a, b);
                        if ordered {
                            assert_eq!(v(a).cmp(&v(b)), i.cmp(&j), "{} vs {}", a, b);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_compare_to() {
        check_groups(
            &[
                &["0", "0.0.0"],
                &["0.1", "0.01"],
                &["0.2", "0.2.0"],
                &["0.2.1"],
                &["1.2alpha"],
                &["1.2alpha1"],
                &["1.2alpha2"],
                &["1.2beta1"],
                &["1.2beta12"],
                &["1.2beta12.1"],
                &["1.2.0", "1.2"],
                &["1.20"],
                &["10", "10.0"],
                &["10.0.0.10A255"],
                &["10.2"],
                &["10.2.0.10P99q"],
                &["10.10.simulator.internal", "10.10"],
            ],
            true,
        );
    }

    #[test]
    fn test_equals() {
        check_groups(
            &[
                &["0", "0.0.0"],
                &["0.1", "0.01"],
                &["0.2", "0.2.0"],
                &["1.2xy2", "1.2xy2"],
                &["10.2.0.10P99q", "10.2.0.10P99q0"],
                &["1.2x", "1.2x0", "1.2x0.0"],
            ],
            false,
        );
    }

    #[test]
    fn test_to_string_with_components() {
        let d = v("42.8");
        assert_eq!(d.to_string_with_components(1), "42");
        assert_eq!(d.to_string_with_components(2), "42.8");
        assert_eq!(d.to_string_with_components(3), "42.8.0");
        assert_eq!(d.to_string_with_components(4), "42.8.0.0");
    }

    #[test]
    fn test_to_string_with_components_trailing_zero() {
        let d = v("4.3alpha3.0");
        assert_eq!(d.to_string_with_components(1), "4");
        assert_eq!(d.to_string_with_components(2), "4.3alpha3");
        assert_eq!(d.to_string_with_components(3), "4.3alpha3.0");
        assert_eq!(d.to_string_with_components(4), "4.3alpha3.0.0");
        assert_eq!(d.to_string_with_components(5), "4.3alpha3.0.0.0");
    }

    #[test]
    fn test_to_string_with_components_zero_component() {
        let d = v("0");
        assert_eq!(d.to_string_with_components(1), "0");
        assert_eq!(d.to_string_with_components(2), "0.0");
        assert_eq!(d.to_string_with_components(3), "0.0.0");
    }

    #[test]
    fn test_to_string_with_minimum_component() {
        let d = v("42.8");
        assert_eq!(d.to_string_with_minimum_components(0), "42.8");
        assert_eq!(d.to_string_with_minimum_components(1), "42.8");
        assert_eq!(d.to_string_with_minimum_components(2), "42.8");
        assert_eq!(d.to_string_with_minimum_components(3), "42.8.0");
        assert_eq!(d.to_string_with_minimum_components(4), "42.8.0.0");
    }

    #[test]
    fn test_to_string_with_minimum_component_trailing_zero() {
        let d = v("4.3alpha3.0");
        assert_eq!(d.to_string_with_minimum_components(0), "4.3alpha3.0");
        assert_eq!(d.to_string_with_minimum_components(1), "4.3alpha3.0");
        assert_eq!(d.to_string_with_minimum_components(2), "4.3alpha3.0");
        assert_eq!(d.to_string_with_minimum_components(3), "4.3alpha3.0");
        assert_eq!(d.to_string_with_minimum_components(4), "4.3alpha3.0.0");
        assert_eq!(d.to_string_with_minimum_components(5), "4.3alpha3.0.0.0");
    }

    #[test]
    fn test_to_string_with_minimum_components_zero_component() {
        let d = v("0");
        assert_eq!(d.to_string_with_minimum_components(0), "0");
        assert_eq!(d.to_string_with_minimum_components(1), "0");
        assert_eq!(d.to_string_with_minimum_components(2), "0.0");
        assert_eq!(d.to_string_with_minimum_components(3), "0.0.0");
    }

    #[test]
    fn test_illegal_version_no_leading_integer() {
        let e = DottedVersion::from_string("a").unwrap_err();
        assert!(e.get_message().contains('a'));
    }

    #[test]
    fn test_illegal_version_empty() {
        assert!(DottedVersion::from_string("").is_err());
    }

    #[test]
    fn test_illegal_version_punctuation() {
        assert!(DottedVersion::from_string("2:3").is_err());
    }

    #[test]
    fn test_illegal_version_empty_component() {
        assert!(DottedVersion::from_string("1..3").is_err());
    }

    #[test]
    fn test_illegal_version_negative_component() {
        assert!(DottedVersion::from_string("1.-1").is_err());
    }

    #[test]
    fn option_equality_is_string_equality() {
        let a = DottedVersion::option(Some(v("1.0"))).unwrap();
        let b = DottedVersion::option(Some(v("1"))).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.get(), b.get());
    }
}
