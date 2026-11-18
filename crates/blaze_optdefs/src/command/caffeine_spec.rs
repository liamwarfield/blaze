//! `CaffeineSpec` values and the `Converters.CaffeineSpecConverter` used by
//! `--directory_creation_cache`.

use super::prelude::*;

/// `com.github.benmanes.caffeine.cache.CaffeineSpec`, holding its specification string.
// PORT: Caffeine is not ported. Only the specification forms that need no Caffeine logic are
// accepted (see CaffeineSpecConverter); equality compares the specification strings.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CaffeineSpec {
    /// `CaffeineSpec.toParsableString()` input.
    pub specification: String,
    /// The `maximumSize=` setting, if given.
    pub maximum_size: Option<i64>,
}

impl OptionScalar for CaffeineSpec {
    fn java_to_string(&self) -> String {
        unported!("CaffeineSpec.toString")
    }
}
option_field_type!(
    CaffeineSpec,
    "com.github.benmanes.caffeine.cache.CaffeineSpec"
);

// Java: com/google/devtools/common/options/Converters.java:709 Converters.CaffeineSpecConverter
/// Converts to a `CaffeineSpec`.
// PORT: replaces blaze_options' CaffeineSpecConverter (which is fully unported) so that the
// default `maximumSize=100000` converts. CaffeineSpec.parse is not ported: inputs other than ""
// and `maximumSize=<digits>` call unported!.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaffeineSpecConverter;

impl Converter for CaffeineSpecConverter {
    // Java: com/google/devtools/common/options/Converters.java:711 Converters.CaffeineSpecConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let spec = non_null(input);
        let maximum_size = if spec.is_empty() {
            None
        } else {
            match spec
                .strip_prefix("maximumSize=")
                .filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|d| d.parse::<i64>().ok())
            {
                Some(n) => Some(n),
                None => unported!("CaffeineSpec.parse", "{}", spec),
            }
        };
        Ok(Some(Value::new(CaffeineSpec {
            specification: spec.to_string(),
            maximum_size,
        })))
    }
    fn get_type_description(&self) -> String {
        "Converts to a CaffeineSpec, or null if the input is empty".into()
    }
}
