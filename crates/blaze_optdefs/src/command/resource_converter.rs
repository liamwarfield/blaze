//! Port of `com/google/devtools/build/lib/util/ResourceConverter.java` and
//! `CpuResourceConverter.java`: converters for resource amounts given as an integer or as a
//! keyword (`auto`, `HOST_CPUS`, `HOST_RAM`) optionally followed by `[-|*]<float>`.

use super::local_host_capacity::get_local_host_capacity;
use super::prelude::*;

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:47 ResourceConverter.AUTO_KEYWORD
pub const AUTO_KEYWORD: &str = "auto";
// Java: com/google/devtools/build/lib/util/ResourceConverter.java:48 ResourceConverter.HOST_CPUS_KEYWORD
pub const HOST_CPUS_KEYWORD: &str = "HOST_CPUS";
// Java: com/google/devtools/build/lib/util/ResourceConverter.java:49 ResourceConverter.HOST_RAM_KEYWORD
pub const HOST_RAM_KEYWORD: &str = "HOST_RAM";

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:146 ResourceConverter.FLAG_SYNTAX
/// Description of the accepted inputs to the converter.
pub const FLAG_SYNTAX: &str = "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), \
                               optionally followed by an operation ([-|*]<float>) eg. \"auto\", \
                               \"HOST_CPUS*.5\"";

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:51 ResourceConverter.HOST_CPUS_SUPPLIER
pub fn host_cpus_supplier() -> i32 {
    get_local_host_capacity().cpu_usage.ceil() as i32
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:53 ResourceConverter.HOST_RAM_SUPPLIER
pub fn host_ram_supplier() -> i32 {
    get_local_host_capacity().memory_mb.ceil() as i32
}

/// Java `Math.round(double)` followed by the `(int)` narrowing cast.
fn java_round_to_int(d: f64) -> i32 {
    // `as i64` saturates and maps NaN to 0, like Math.round; `as i32` on an i64 wraps like the
    // Java narrowing conversion.
    ((d + 0.5).floor() as i64) as i32
}

/// Guava `Ints.tryParse(String)`: an optional `-` and decimal digits.
fn ints_try_parse(s: &str) -> Option<i32> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok()
}

/// Whether `c` is a Java regex line terminator (not matched by `.`).
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}')
}

/// Matches `(?:.[0-9]+)` against all of `s`.
fn dot_digits(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if !is_line_terminator(c) => {
            let rest = chars.as_str();
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
        }
        _ => false,
    }
}

/// Matches `[-*][0-9]?(?:.[0-9]+)?` against all of `s` (Java's `validInputPattern` expression
/// group, built from the `OPERATORS` keys `-` and `*`).
fn is_expression(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('-').or_else(|| s.strip_prefix('*')) else {
        return false;
    };
    if rest.is_empty() || dot_digits(rest) {
        return true;
    }
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_ascii_digit() => {
            let after = chars.as_str();
            after.is_empty() || dot_digits(after)
        }
        _ => false,
    }
}

/// Java `String.format("%f", d)`.
fn java_f(d: f64) -> String {
    format!("{:.6}", d)
}

/// A keyword and the supplier of its value (Java `ImmutableMap<String, Supplier<T>>` entry).
pub type Keyword<T> = (&'static str, fn() -> T);

/// A binary operator of `OPERATORS`.
type Operator = (&'static str, fn(f64, f64) -> f64);

/// The keyword part shared by the integer and double converters.
#[derive(Clone, Debug)]
struct Keywords<T: Copy + 'static> {
    keywords: Vec<Keyword<T>>,
}

impl<T: Copy + Into<f64>> Keywords<T> {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:183 ResourceConverter.convertKeyword
    fn convert_keyword(
        &self,
        input: &str,
        type_description: &str,
    ) -> Result<f64, OptionsParsingException> {
        // Java: validInputPattern =
        //   (?<keyword>k1|k2|..)(?<expression>[-*][0-9]?(?:.[0-9]+)?)? , matched against all input.
        for (keyword, supplier) in &self.keywords {
            if let Some(rest) = input.strip_prefix(keyword) {
                if rest.is_empty() {
                    return Self::apply_operator(None, *supplier);
                }
                if is_expression(rest) {
                    return Self::apply_operator(Some(rest), *supplier);
                }
            }
        }
        Err(OptionsParsingException::new(format!(
            "Parameter '{}' does not follow correct syntax. This flag takes {}.",
            input, type_description
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:198 ResourceConverter.applyOperator
    fn apply_operator(
        expression: Option<&str>,
        first_operand_supplier: fn() -> T,
    ) -> Result<f64, OptionsParsingException> {
        let Some(expression) = expression else {
            return Ok(first_operand_supplier().into());
        };
        // Java: OPERATORS = {"-": l - r, "*": l * r}, in this order.
        let operators: [Operator; 2] = [("-", |l, r| l - r), ("*", |l, r| l * r)];
        for (op, apply) in operators {
            if let Some(operand) = expression.strip_prefix(op) {
                // Java: Float.parseFloat; the pattern only lets through digits and one separator.
                let second_operand: f32 = match operand.parse::<f32>() {
                    Ok(f) => f,
                    Err(_) => {
                        return Err(OptionsParsingException::new(format!(
                            "'{} is not a float",
                            operand
                        )));
                    }
                };
                return Ok(apply(
                    first_operand_supplier().into(),
                    second_operand as f64,
                ));
            }
        }
        // This should never happen because we've checked for a valid operator already.
        Err(OptionsParsingException::new(format!(
            "Parameter value '{}' does not contain a valid operator.",
            expression
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:244 ResourceConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        let first_keyword = self.keywords[0].0;
        let names: Vec<&str> = self.keywords.iter().map(|(k, _)| *k).collect();
        format!(
            "an integer, or a keyword (\"{}\"), optionally followed by an operation \
             ([-|*]<float>) eg. \"{}\", \"{}*.5\"",
            names.join("\", \""),
            first_keyword,
            HOST_CPUS_KEYWORD
        )
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:83 ResourceConverter.IntegerConverter
/// Resource converter for integers. Subclasses that override `checkAndLimit` or
/// `getTypeDescription` in Java pass replacements for them.
#[derive(Clone, Debug)]
pub struct ResourceIntegerConverter {
    keywords: Keywords<i32>,
    pub min_value: i32,
    pub max_value: i32,
    check_and_limit: fn(&ResourceIntegerConverter, i32) -> Result<i32, OptionsParsingException>,
    type_description: Option<fn() -> String>,
}

impl ResourceIntegerConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:86 ResourceConverter.IntegerConverter.<init>
    /// Keywords `auto` (the given supplier), `HOST_CPUS` and `HOST_RAM`.
    pub fn new(auto: fn() -> i32, min_value: i32, max_value: i32) -> Self {
        Self::with_keywords(
            vec![
                (AUTO_KEYWORD, auto),
                (HOST_CPUS_KEYWORD, host_cpus_supplier),
                (HOST_RAM_KEYWORD, host_ram_supplier),
            ],
            min_value,
            max_value,
        )
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:99 ResourceConverter.IntegerConverter.<init>
    pub fn with_keywords(keywords: Vec<Keyword<i32>>, min_value: i32, max_value: i32) -> Self {
        ResourceIntegerConverter {
            keywords: Keywords { keywords },
            min_value,
            max_value,
            check_and_limit: Self::default_check_and_limit,
            type_description: None,
        }
    }

    /// Replaces `checkAndLimit` (a Java subclass override).
    pub fn with_check_and_limit(
        mut self,
        f: fn(&ResourceIntegerConverter, i32) -> Result<i32, OptionsParsingException>,
    ) -> Self {
        self.check_and_limit = f;
        self
    }

    /// Replaces `getTypeDescription` (a Java subclass override).
    pub fn with_type_description(mut self, f: fn() -> String) -> Self {
        self.type_description = Some(f);
        self
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:228 ResourceConverter.checkAndLimit
    pub fn default_check_and_limit(&self, value: i32) -> Result<i32, OptionsParsingException> {
        if value < self.min_value {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' must be at least {}.",
                java_f(value as f64),
                java_f(self.min_value as f64)
            )));
        }
        if value > self.max_value {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' cannot be greater than {}.",
                java_f(value as f64),
                java_f(self.max_value as f64)
            )));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:106 ResourceConverter.IntegerConverter.convert
    pub fn convert_int(&self, input: &str) -> Result<i32, OptionsParsingException> {
        if ints_try_parse(input).is_some() {
            let v = blaze_options::IntegerConverter
                .convert(Some(input), None)?
                .expect("IntegerConverter never returns null")
                .get::<i32>();
            (self.check_and_limit)(self, v)
        } else {
            let d = self
                .keywords
                .convert_keyword(input, &self.get_type_description())?;
            (self.check_and_limit)(self, java_round_to_int(d))
        }
    }
}

impl Converter for ResourceIntegerConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_int(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        match self.type_description {
            Some(f) => f(),
            None => self.keywords.get_type_description(),
        }
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:113 ResourceConverter.DoubleConverter
/// Resource converter for doubles.
#[derive(Clone, Debug)]
pub struct ResourceDoubleConverter {
    keywords: Keywords<f64>,
    pub min_value: f64,
    pub max_value: f64,
}

impl ResourceDoubleConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:116 ResourceConverter.DoubleConverter.<init>
    /// Keywords `auto` (the given supplier), `HOST_CPUS` and `HOST_RAM`.
    pub fn new(auto: fn() -> f64, min_value: f64, max_value: f64) -> Self {
        Self::with_keywords(
            vec![
                (AUTO_KEYWORD, auto),
                (HOST_CPUS_KEYWORD, || host_cpus_supplier() as f64),
                (HOST_RAM_KEYWORD, || host_ram_supplier() as f64),
            ],
            min_value,
            max_value,
        )
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:126 ResourceConverter.DoubleConverter.<init>
    pub fn with_keywords(keywords: Vec<Keyword<f64>>, min_value: f64, max_value: f64) -> Self {
        ResourceDoubleConverter {
            keywords: Keywords { keywords },
            min_value,
            max_value,
        }
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:228 ResourceConverter.checkAndLimit
    pub fn check_and_limit(&self, value: f64) -> Result<f64, OptionsParsingException> {
        // Java: Double.compareTo.
        if value.total_cmp(&self.min_value).is_lt() {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' must be at least {}.",
                java_f(value),
                java_f(self.min_value)
            )));
        }
        if value.total_cmp(&self.max_value).is_gt() {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' cannot be greater than {}.",
                java_f(value),
                java_f(self.max_value)
            )));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:133 ResourceConverter.DoubleConverter.convert
    pub fn convert_double(&self, input: &str) -> Result<f64, OptionsParsingException> {
        // Java: Doubles.tryParse(input) != null.
        if blaze_options::java_parse_double(input).is_some() {
            let v = blaze_options::DoubleConverter
                .convert(Some(input), None)?
                .expect("DoubleConverter never returns null")
                .get::<f64>();
            self.check_and_limit(v)
        } else {
            self.keywords
                .convert_keyword(input, &self.keywords.get_type_description())
        }
    }

    pub fn get_type_description(&self) -> String {
        self.keywords.get_type_description()
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:57 ResourceConverter.AssignmentConverter
/// Resource converter for assignments.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceConverterAssignmentConverter;

impl ResourceConverterAssignmentConverter {
    fn resource() -> ResourceDoubleConverter {
        ResourceDoubleConverter::with_keywords(
            vec![
                (HOST_CPUS_KEYWORD, || host_cpus_supplier() as f64),
                (HOST_RAM_KEYWORD, || host_ram_supplier() as f64),
            ],
            0.0,
            f64::MAX,
        )
    }
}

impl Converter for ResourceConverterAssignmentConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:71 ResourceConverter.AssignmentConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let (key, value) = AssignmentConverter.convert_entry(non_null(input))?;
        Ok(Some(Value::new((
            key,
            Self::resource().convert_double(&value)?,
        ))))
    }
    fn get_type_description(&self) -> String {
        format!(
            "a named double, 'name=value', where value is {}",
            Self::resource().get_type_description()
        )
    }
}

// Java: com/google/devtools/build/lib/util/CpuResourceConverter.java:21 CpuResourceConverter
/// Converter for --local_resources=cpu=, which takes an integer greater than or equal to 1, or
/// "HOST_CPUS", optionally followed by [-|*]<float>.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuResourceConverter;

impl CpuResourceConverter {
    fn inner() -> ResourceIntegerConverter {
        ResourceIntegerConverter::with_keywords(
            vec![(HOST_CPUS_KEYWORD, host_cpus_supplier)],
            0,
            i32::MAX,
        )
        .with_type_description(|| {
            format!(
                "an integer, or \"{}\", optionally followed by [-|*]<float>.",
                HOST_CPUS_KEYWORD
            )
        })
    }
}

impl Converter for CpuResourceConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        Self::inner().convert(input, c)
    }
    fn get_type_description(&self) -> String {
        Self::inner().get_type_description()
    }
}

/// Declares a unit-struct converter that delegates to a [`ResourceIntegerConverter`] (a Java
/// `ResourceConverter.IntegerConverter` subclass).
macro_rules! resource_integer_converter {
    ($(#[$m:meta])* $vis:vis $name:ident => $make:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default)]
        $vis struct $name;

        impl $name {
            pub fn inner() -> $crate::command::resource_converter::ResourceIntegerConverter {
                $make
            }
        }

        impl blaze_options::Converter for $name {
            fn convert(
                &self,
                input: Option<&str>,
                c: Option<&blaze_options::ConversionContext>,
            ) -> Result<Option<blaze_options::Value>, blaze_options::OptionsParsingException> {
                Self::inner().convert(input, c)
            }
            fn get_type_description(&self) -> String {
                Self::inner().get_type_description()
            }
        }
    };
}
pub(crate) use resource_integer_converter;

#[cfg(test)]
mod tests {
    use super::super::local_host_capacity::{LocalHostResources, set_local_host_capacity};
    use super::*;

    fn none() -> i32 {
        // Java tests pass `() -> null`; the supplier is never called.
        unreachable!()
    }

    fn five() -> i32 {
        5
    }

    fn capacity(memory_mb: f64, cpu_usage: f64) {
        set_local_host_capacity(Some(LocalHostResources {
            memory_mb,
            cpu_usage,
        }));
    }

    // Ported from src/test/java/com/google/devtools/build/lib/util/ResourceConverterTest.java.

    #[test]
    fn convert_number_returns_int() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        assert_eq!(c.convert_int("6").unwrap(), 6);
    }

    #[test]
    fn convert_number_returns_double() {
        let c = ResourceDoubleConverter::new(|| unreachable!(), 1.0, f64::MAX);
        assert_eq!(c.convert_double("6.3").unwrap(), 6.3);
    }

    #[test]
    fn convert_number_greater_than_max_throws_exception() {
        let c = ResourceIntegerConverter::new(none, 0, 1);
        let thrown = c.convert_int("2").unwrap_err();
        assert!(thrown.get_message().contains("cannot be greater than 1"));
    }

    #[test]
    fn convert_number_less_than_min_throws_exception() {
        let c = ResourceIntegerConverter::new(none, -1, 1);
        assert_eq!(c.convert_int("0").unwrap(), 0);
        let thrown = c.convert_int("-2").unwrap_err();
        assert!(thrown.get_message().contains("must be at least -1"));
    }

    #[test]
    fn convert_auto_returns_supplied_auto_value() {
        let c = ResourceIntegerConverter::new(five, 1, i32::MAX);
        assert_eq!(c.convert_int("auto").unwrap(), 5);
    }

    #[test]
    fn convert_auto_with_operator_applies_operator_to_auto() {
        let c = ResourceIntegerConverter::new(five, 1, i32::MAX);
        assert_eq!(c.convert_int("auto-1").unwrap(), 4);
    }

    #[test]
    fn convert_auto_with_invalid_operator_throws_exception() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        let thrown = c.convert_int("auto/2").unwrap_err();
        assert!(
            thrown
                .get_message()
                .contains("does not follow correct syntax")
        );
    }

    #[test]
    fn convert_auto_is_float_returns_rounded_int() {
        let c = ResourceIntegerConverter::new(five, 1, i32::MAX);
        assert_eq!(c.convert_int("auto*.51").unwrap(), 3);
    }

    #[test]
    fn convert_host_cpus_returns_cpu_setting() {
        capacity(1.0, 15.0);
        let c = ResourceIntegerConverter::new(five, 1, i32::MAX);
        assert_eq!(c.convert_int("HOST_CPUS").unwrap(), 15);
        set_local_host_capacity(None);
    }

    #[test]
    fn convert_ram_returns_ram_setting() {
        capacity(10.0, 0.0);
        let c = ResourceIntegerConverter::new(five, 1, i32::MAX);
        assert_eq!(c.convert_int("HOST_RAM").unwrap(), 10);
        set_local_host_capacity(None);
    }

    #[test]
    fn convert_float_throws_exception() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        let thrown = c.convert_int(".5").unwrap_err();
        assert!(thrown.get_message().contains("This flag takes an integer"));
    }

    #[test]
    fn convert_wrong_keyword_throws_exception() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        let thrown = c.convert_int("invalid_keyword").unwrap_err();
        assert_eq!(
            thrown.get_message(),
            "Parameter 'invalid_keyword' does not follow correct syntax. This flag takes an \
             integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed by \
             an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"."
        );
    }

    #[test]
    fn convert_almost_valid_keyword_throws_exception() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        let thrown = c.convert_int("aut").unwrap_err();
        assert!(
            thrown
                .get_message()
                .contains("does not follow correct syntax")
        );
    }

    #[test]
    fn build_converter_before_resources_uses_resources() {
        let c = ResourceIntegerConverter::new(none, 1, i32::MAX);
        capacity(0.0, 15.0);
        assert_eq!(c.convert_int("HOST_CPUS").unwrap(), 15);
        set_local_host_capacity(None);
    }

    // Ported from src/test/java/com/google/devtools/build/lib/util/CpuResourceConverterTest.java.

    #[test]
    fn test_constructor_correct_min_and_max_values() {
        let c = CpuResourceConverter::inner();
        assert_eq!(c.min_value, 0);
        assert_eq!(c.max_value, i32::MAX);
    }

    // Blaze-specific checks.

    #[test]
    fn keyword_operators_and_descriptions() {
        let c = ResourceIntegerConverter::new(|| 7, 1, 100);
        assert_eq!(c.convert_int("auto*1.5").unwrap(), 11);
        assert_eq!(
            c.convert_int("0").unwrap_err().get_message(),
            "Value '(0.000000)' must be at least 1.000000."
        );
        assert_eq!(c.get_type_description(), FLAG_SYNTAX);
        assert_eq!(
            c.convert_int("auto-").unwrap_err().get_message(),
            "' is not a float"
        );
        assert!(host_cpus_supplier() >= 1);
        assert!(host_ram_supplier() >= 1);
    }

    #[test]
    fn assignment() {
        let v = ResourceConverterAssignmentConverter
            .convert(Some("cpu=4"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.get::<(String, f64)>(), ("cpu".to_string(), 4.0));
        assert_eq!(
            ResourceConverterAssignmentConverter.get_type_description(),
            "a named double, 'name=value', where value is an integer, or a keyword (\"HOST_CPUS\", \
             \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \"HOST_CPUS\", \
             \"HOST_CPUS*.5\""
        );
        assert_eq!(
            CpuResourceConverter.get_type_description(),
            "an integer, or \"HOST_CPUS\", optionally followed by [-|*]<float>."
        );
    }
}
