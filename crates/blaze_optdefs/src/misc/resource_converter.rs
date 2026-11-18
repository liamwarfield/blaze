//! Port of `lib/util/ResourceConverter.java` and `lib/util/RamResourceConverter.java`.

use std::cmp::Ordering;
use std::sync::{Arc, RwLock};

use blaze_options::{
    AssignmentConverter, ConversionContext, Converter, DoubleConverter, IntegerConverter,
    OptionsParsingException, Value, converter::non_null,
};

type ConvResult = Result<Option<Value>, OptionsParsingException>;

/// Java `Supplier<T>`.
pub type Supplier<T> = Arc<dyn Fn() -> T + Send + Sync>;

pub const AUTO_KEYWORD: &str = "auto";
pub const HOST_CPUS_KEYWORD: &str = "HOST_CPUS";
pub const HOST_RAM_KEYWORD: &str = "HOST_RAM";

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:146 ResourceConverter.FLAG_SYNTAX
/// Description of the accepted inputs to the converter.
pub const FLAG_SYNTAX: &str = "an integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), \
                               optionally followed by an operation ([-|*]<float>) eg. \"auto\", \
                               \"HOST_CPUS*.5\"";

/// The part of `actions/LocalHostCapacity.java` the suppliers read: the host's
/// `(memoryMb, cpuUsage)` (`ResourceSet.getMemoryMb()`, `ResourceSet.getCpuUsage()`).
///
/// PORT: `LocalHostResource.get()` (the JVM's container-aware total physical memory and
/// `Runtime.availableProcessors()`) is not ported; until it is, only a capacity installed with
/// [`set_local_host_capacity`] (Java's `@VisibleForTesting` setter) is available.
static LOCAL_HOST_CAPACITY: RwLock<Option<(f64, f64)>> = RwLock::new(None);

// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:32 LocalHostCapacity.getLocalHostCapacity
fn get_local_host_capacity() -> (f64, f64) {
    match *LOCAL_HOST_CAPACITY
        .read()
        .unwrap_or_else(|e| e.into_inner())
    {
        Some(c) => c,
        None => blaze_util::unported!("LocalHostResource.get"),
    }
}

// Java: com/google/devtools/build/lib/actions/LocalHostCapacity.java:53 LocalHostCapacity.setLocalHostCapacity
/// Sets the local host capacity to hardcoded `(memory_mb, cpu_usage)` values, or `None` to use the
/// machine-specific values again.
pub fn set_local_host_capacity(capacity: Option<(f64, f64)>) {
    *LOCAL_HOST_CAPACITY
        .write()
        .unwrap_or_else(|e| e.into_inner()) = capacity;
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:51 ResourceConverter.HOST_CPUS_SUPPLIER
pub fn host_cpus_supplier() -> i32 {
    // (int) Math.ceil(LocalHostCapacity.getLocalHostCapacity().getCpuUsage())
    get_local_host_capacity().1.ceil() as i32
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:53 ResourceConverter.HOST_RAM_SUPPLIER
pub fn host_ram_supplier() -> i32 {
    // (int) Math.ceil(LocalHostCapacity.getLocalHostCapacity().getMemoryMb())
    get_local_host_capacity().0.ceil() as i32
}

/// The numeric types a [`ResourceConverter`] produces (`T extends Number & Comparable<T>`).
pub trait ResourceNumber: Copy + std::fmt::Debug + Send + Sync + 'static {
    /// `Number.doubleValue()`.
    fn double_value(self) -> f64;
    /// `Comparable.compareTo`.
    fn java_compare(self, other: Self) -> Ordering;
}

impl ResourceNumber for i32 {
    fn double_value(self) -> f64 {
        self as f64
    }
    fn java_compare(self, other: Self) -> Ordering {
        self.cmp(&other)
    }
}

impl ResourceNumber for f64 {
    fn double_value(self) -> f64 {
        self
    }
    // Java: Double.compare (NaN is greater than everything, -0.0 < 0.0).
    fn java_compare(self, other: Self) -> Ordering {
        match (self.is_nan(), other.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => self.total_cmp(&other),
        }
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:45 ResourceConverter
/// Converter for options that configure Bazel's resource usage.
///
/// The option can take either a value or one of the keywords `auto`, `HOST_CPUS`, or `HOST_RAM`,
/// followed by an optional operator in the form `[-|*]<float>`.
pub struct ResourceConverter<T: ResourceNumber> {
    /// `ImmutableMap<String, Supplier<T>>`, in insertion order.
    keywords: Vec<(&'static str, Supplier<T>)>,
    pub min_value: T,
    pub max_value: T,
    /// Subclasses overriding `getTypeDescription()` (`RamResourceConverter`).
    type_description_override: Option<String>,
}

impl<T: ResourceNumber> ResourceConverter<T> {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:172 ResourceConverter.<init>
    pub fn new(keywords: Vec<(&'static str, Supplier<T>)>, min_value: T, max_value: T) -> Self {
        ResourceConverter {
            keywords,
            min_value,
            max_value,
            type_description_override: None,
        }
    }

    /// Matches `validInputPattern`:
    /// `(?<keyword>k1|k2|...)(?<expression>[-*][0-9]?(?:.[0-9]+)?)?`, against the whole input.
    fn match_valid_input<'a>(&self, input: &'a str) -> Option<(&'static str, Option<&'a str>)> {
        for (keyword, _) in &self.keywords {
            if let Some(rest) = input.strip_prefix(keyword) {
                if rest.is_empty() {
                    return Some((keyword, None));
                }
                if expression_matches(rest) {
                    return Some((keyword, Some(rest)));
                }
            }
        }
        None
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:183 ResourceConverter.convertKeyword
    pub fn convert_keyword(&self, input: &str) -> Result<f64, OptionsParsingException> {
        if let Some((keyword, expression)) = self.match_valid_input(input)
            && let Some((_, resource_supplier)) = self.keywords.iter().find(|(k, _)| *k == keyword)
        {
            return self.apply_operator(expression, resource_supplier);
        }
        Err(OptionsParsingException::new(format!(
            "Parameter '{}' does not follow correct syntax. This flag takes {}.",
            input,
            self.get_type_description()
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:198 ResourceConverter.applyOperator
    /// Applies function designated in `expression` (`[-|*]<float>`) to value.
    fn apply_operator(
        &self,
        expression: Option<&str>,
        first_operand_supplier: &Supplier<T>,
    ) -> Result<f64, OptionsParsingException> {
        let Some(expression) = expression else {
            return Ok(first_operand_supplier().double_value());
        };
        // OPERATORS: ImmutableMap {"-": l - r, "*": l * r}.
        for operator in ["-", "*"] {
            if let Some(operand) = expression.strip_prefix(operator) {
                let second_operand = match java_parse_float(operand) {
                    Some(f) => f,
                    None => {
                        return Err(OptionsParsingException::with_cause(
                            format!("'{} is not a float", operand),
                            java_number_format_message(operand),
                        ));
                    }
                };
                let l = first_operand_supplier().double_value();
                let r = second_operand as f64;
                return Ok(if operator == "-" { l - r } else { l * r });
            }
        }
        // This should never happen because we've checked for a valid operator already.
        Err(OptionsParsingException::new(format!(
            "Parameter value '{}' does not contain a valid operator.",
            expression
        )))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:228 ResourceConverter.checkAndLimit
    /// Checks validity of a resource value against min/max constraints.
    pub fn check_and_limit(&self, value: T) -> Result<T, OptionsParsingException> {
        if value.java_compare(self.min_value) == Ordering::Less {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' must be at least {}.",
                java_format_f(value.double_value()),
                java_format_f(self.min_value.double_value())
            )));
        }
        if value.java_compare(self.max_value) == Ordering::Greater {
            return Err(OptionsParsingException::new(format!(
                "Value '({})' cannot be greater than {}.",
                java_format_f(value.double_value()),
                java_format_f(self.max_value.double_value())
            )));
        }
        Ok(value)
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:244 ResourceConverter.getTypeDescription
    pub fn get_type_description(&self) -> String {
        if let Some(d) = &self.type_description_override {
            return d.clone();
        }
        let first_keyword = self.keywords[0].0;
        let keys: Vec<&str> = self.keywords.iter().map(|(k, _)| *k).collect();
        format!(
            "an integer, or a keyword (\"{}\"), optionally followed by an operation \
             ([-|*]<float>) eg. \"{}\", \"{}*.5\"",
            keys.join("\", \""),
            first_keyword,
            HOST_CPUS_KEYWORD
        )
    }
}

/// `[-*][0-9]?(?:.[0-9]+)?` against the whole string.
fn expression_matches(s: &str) -> bool {
    let Some(tail) = s.strip_prefix(['-', '*']) else {
        return false;
    };
    // `.` matches any character except a line terminator.
    fn dot_digits(t: &str) -> bool {
        if t.is_empty() {
            return true;
        }
        let mut it = t.chars();
        match it.next() {
            Some(c) if !matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}') => {
                let r = it.as_str();
                !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit())
            }
            _ => false,
        }
    }
    if tail.as_bytes().first().is_some_and(|b| b.is_ascii_digit()) && dot_digits(&tail[1..]) {
        return true;
    }
    dot_digits(tail)
}

/// Java `Float.parseFloat` for the inputs `validInputPattern` lets through (one optional digit,
/// then optionally one arbitrary character followed by digits).
fn java_parse_float(s: &str) -> Option<f32> {
    let t = s.trim_matches(|c: char| c <= ' ');
    if t.is_empty() {
        return None;
    }
    // Rust accepts "inf"/"nan" spellings Java rejects; the pattern cannot produce them, but keep
    // the check so this never accepts more than Java.
    if t.bytes()
        .any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E')
    {
        return None;
    }
    t.parse::<f32>().ok()
}

/// `NumberFormatException.getMessage()` of `Float.parseFloat`.
fn java_number_format_message(s: &str) -> String {
    if s.trim_matches(|c: char| c <= ' ').is_empty() {
        "empty String".to_string()
    } else {
        format!("For input string: \"{}\"", s)
    }
}

/// Guava `Ints.tryParse(String)`: an optional '-', then ASCII digits, within `int` range.
pub fn guava_ints_try_parse(s: &str) -> Option<i32> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok()
}

/// Guava `Doubles.tryParse(String)`: matches `FLOATING_POINT_PATTERN`, then `Double.parseDouble`.
pub fn guava_doubles_try_parse(s: &str) -> Option<f64> {
    // Guava's pattern: [+-]?(?:NaN|Infinity|(decimal)|(hex)) where
    // decimal = (?:\d+#(?:\.\d*#)?|\.\d+#)(?:[eE][+-]?\d+#)?[fFdD]?
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    let negative = s.starts_with('-');
    match unsigned {
        "NaN" => return Some(f64::NAN),
        "Infinity" => {
            return Some(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        _ => {}
    }
    if unsigned.starts_with("0x") || unsigned.starts_with("0X") {
        blaze_util::unported!("Doubles.tryParse (hexadecimal floating point)");
    }
    let body = unsigned
        .strip_suffix(['f', 'F', 'd', 'D'])
        .unwrap_or(unsigned);
    let b = body.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - fs;
    }
    if int_digits == 0 && frac_digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == es {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    let v: f64 = body.parse().ok()?;
    Some(if negative { -v } else { v })
}

/// Java `Math.round(double)` (ties towards positive infinity, saturating, NaN is 0).
pub fn java_math_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}

/// Java `String.format("%f", d)`: six fraction digits, rounded half-up from the shortest decimal
/// representation (Java's `FormattedFloatingDecimal`).
pub fn java_format_f(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // Rust's `{}` prints the shortest round-trip digits without an exponent.
    let s = format!("{}", d);
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, s),
    };
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i.to_string(), f.to_string()),
        None => (s.clone(), String::new()),
    };
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).collect();
    let int_len = int_part.len();
    let mut frac_len = frac_part.len();
    if frac_len > 6 {
        let round_up = digits[int_len + 6] >= b'5';
        digits.truncate(int_len + 6);
        frac_len = 6;
        if round_up {
            let mut k = digits.len();
            loop {
                if k == 0 {
                    digits.insert(0, b'1');
                    break;
                }
                k -= 1;
                if digits[k] == b'9' {
                    digits[k] = b'0';
                } else {
                    digits[k] += 1;
                    break;
                }
            }
        }
    }
    let total_int = digits.len() - frac_len;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    out.push_str(std::str::from_utf8(&digits[..total_int]).unwrap());
    out.push('.');
    out.push_str(std::str::from_utf8(&digits[total_int..]).unwrap());
    for _ in frac_len..6 {
        out.push('0');
    }
    out
}

/// `ImmutableMap.of(AUTO_KEYWORD, auto, HOST_CPUS_KEYWORD, HOST_CPUS_SUPPLIER, HOST_RAM_KEYWORD,
/// HOST_RAM_SUPPLIER)`.
fn default_int_keywords(auto: Supplier<i32>) -> Vec<(&'static str, Supplier<i32>)> {
    vec![
        (AUTO_KEYWORD, auto),
        (HOST_CPUS_KEYWORD, Arc::new(host_cpus_supplier)),
        (HOST_RAM_KEYWORD, Arc::new(host_ram_supplier)),
    ]
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:83 ResourceConverter.IntegerConverter
/// Resource converter for integers.
pub struct ResourceConverterIntegerConverter(pub ResourceConverter<i32>);

impl ResourceConverterIntegerConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:86 ResourceConverter.IntegerConverter.<init>
    pub fn new(auto: Supplier<i32>, min_value: i32, max_value: i32) -> Self {
        Self::with_keywords(default_int_keywords(auto), min_value, max_value)
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:99 ResourceConverter.IntegerConverter.<init>
    pub fn with_keywords(
        keywords: Vec<(&'static str, Supplier<i32>)>,
        min_value: i32,
        max_value: i32,
    ) -> Self {
        ResourceConverterIntegerConverter(ResourceConverter::new(keywords, min_value, max_value))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:105 ResourceConverter.IntegerConverter.convert
    pub fn convert_int(&self, input: &str) -> Result<i32, OptionsParsingException> {
        if guava_ints_try_parse(input).is_some() {
            let v = IntegerConverter
                .convert(Some(input), None)?
                .expect("IntegerConverter returned null")
                .get::<i32>();
            self.0.check_and_limit(v)
        } else {
            // (int) Math.round(...): long to int narrowing.
            self.0
                .check_and_limit(java_math_round(self.0.convert_keyword(input)?) as i32)
        }
    }
}

impl Converter for ResourceConverterIntegerConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_int(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        self.0.get_type_description()
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:113 ResourceConverter.DoubleConverter
/// Resource converter for doubles.
pub struct ResourceConverterDoubleConverter(pub ResourceConverter<f64>);

impl ResourceConverterDoubleConverter {
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:116 ResourceConverter.DoubleConverter.<init>
    pub fn new(auto: Supplier<f64>, min_value: f64, max_value: f64) -> Self {
        Self::with_keywords(
            vec![
                (AUTO_KEYWORD, auto),
                (HOST_CPUS_KEYWORD, Arc::new(|| host_cpus_supplier() as f64)),
                (HOST_RAM_KEYWORD, Arc::new(|| host_ram_supplier() as f64)),
            ],
            min_value,
            max_value,
        )
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:126 ResourceConverter.DoubleConverter.<init>
    pub fn with_keywords(
        keywords: Vec<(&'static str, Supplier<f64>)>,
        min_value: f64,
        max_value: f64,
    ) -> Self {
        ResourceConverterDoubleConverter(ResourceConverter::new(keywords, min_value, max_value))
    }

    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:132 ResourceConverter.DoubleConverter.convert
    pub fn convert_double(&self, input: &str) -> Result<f64, OptionsParsingException> {
        if guava_doubles_try_parse(input).is_some() {
            let v = DoubleConverter
                .convert(Some(input), None)?
                .expect("DoubleConverter returned null")
                .get::<f64>();
            self.0.check_and_limit(v)
        } else {
            self.0.convert_keyword(input)
        }
    }
}

impl Converter for ResourceConverterDoubleConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(self.convert_double(non_null(input))?)))
    }
    fn get_type_description(&self) -> String {
        self.0.get_type_description()
    }
}

// Java: com/google/devtools/build/lib/util/ResourceConverter.java:57 ResourceConverter.AssignmentConverter
/// Resource converter for assignments.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceConverterAssignmentConverter;

impl ResourceConverterAssignmentConverter {
    fn resource() -> ResourceConverterDoubleConverter {
        ResourceConverterDoubleConverter::with_keywords(
            vec![
                (HOST_CPUS_KEYWORD, Arc::new(|| host_cpus_supplier() as f64)),
                (HOST_RAM_KEYWORD, Arc::new(|| host_ram_supplier() as f64)),
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
        let d = Self::resource().convert_double(&value)?;
        Ok(Some(Value::new((key, d))))
    }
    // Java: com/google/devtools/build/lib/util/ResourceConverter.java:77 ResourceConverter.AssignmentConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        format!(
            "a named double, 'name=value', where value is {}",
            Self::resource().get_type_description()
        )
    }
}

// Java: com/google/devtools/build/lib/util/RamResourceConverter.java:22 RamResourceConverter
/// Converter for --local_resources=memory=, which takes an integer greater than or equal to 0, or
/// "HOST_RAM", optionally followed by [-|*]<float>.
#[derive(Clone, Copy, Debug, Default)]
pub struct RamResourceConverter;

impl RamResourceConverter {
    // Java: com/google/devtools/build/lib/util/RamResourceConverter.java:23 RamResourceConverter.<init>
    pub fn inner() -> ResourceConverterIntegerConverter {
        let mut c = ResourceConverterIntegerConverter::with_keywords(
            vec![(HOST_RAM_KEYWORD, Arc::new(host_ram_supplier))],
            0,
            i32::MAX,
        );
        c.0.type_description_override = Some(Self::type_description());
        c
    }

    // Java: com/google/devtools/build/lib/util/RamResourceConverter.java:31 RamResourceConverter.getTypeDescription
    fn type_description() -> String {
        format!(
            "an integer number of MBs, or \"{}\", optionally followed by [-|*]<float>.",
            HOST_RAM_KEYWORD
        )
    }
}

impl Converter for RamResourceConverter {
    fn convert(&self, input: Option<&str>, c: Option<&ConversionContext>) -> ConvResult {
        Self::inner().convert(input, c)
    }
    fn get_type_description(&self) -> String {
        Self::type_description()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int_conv(auto: i32) -> ResourceConverterIntegerConverter {
        ResourceConverterIntegerConverter::with_keywords(
            vec![
                (AUTO_KEYWORD, Arc::new(move || auto)),
                (HOST_CPUS_KEYWORD, Arc::new(|| 8)),
                (HOST_RAM_KEYWORD, Arc::new(|| 4096)),
            ],
            0,
            i32::MAX,
        )
    }

    #[test]
    fn flag_syntax_matches_default_type_description() {
        assert_eq!(
            int_conv(1).get_type_description(),
            FLAG_SYNTAX,
            "FLAG_SYNTAX and getTypeDescription() agree for the default keyword set"
        );
    }

    #[test]
    fn converts_integers_and_keywords() {
        let c = int_conv(3);
        assert_eq!(c.convert_int("12").unwrap(), 12);
        assert_eq!(c.convert_int("auto").unwrap(), 3);
        assert_eq!(c.convert_int("HOST_CPUS").unwrap(), 8);
        assert_eq!(c.convert_int("HOST_CPUS*.5").unwrap(), 4);
        assert_eq!(c.convert_int("HOST_CPUS-1").unwrap(), 7);
        assert_eq!(c.convert_int("HOST_RAM*0.25").unwrap(), 1024);
    }

    #[test]
    fn rejects_bad_input() {
        let c = int_conv(3);
        assert_eq!(
            c.convert_int("-1").unwrap_err().get_message(),
            "Value '(-1.000000)' must be at least 0.000000."
        );
        assert_eq!(
            c.convert_int("bogus").unwrap_err().get_message(),
            format!(
                "Parameter 'bogus' does not follow correct syntax. This flag takes {}.",
                FLAG_SYNTAX
            )
        );
        assert_eq!(
            c.convert_int("HOST_CPUS*").unwrap_err().get_message(),
            "' is not a float"
        );
    }

    #[test]
    fn ram_resource_converter_description() {
        assert_eq!(
            RamResourceConverter.get_type_description(),
            "an integer number of MBs, or \"HOST_RAM\", optionally followed by [-|*]<float>."
        );
        assert_eq!(
            RamResourceConverter
                .convert(Some("100"), None)
                .unwrap()
                .unwrap()
                .get::<i32>(),
            100
        );
        assert_eq!(
            RamResourceConverter
                .convert(Some("auto"), None)
                .unwrap_err()
                .get_message(),
            "Parameter 'auto' does not follow correct syntax. This flag takes an integer number \
             of MBs, or \"HOST_RAM\", optionally followed by [-|*]<float>.."
        );
    }

    #[test]
    fn java_format_f_cases() {
        assert_eq!(java_format_f(0.0), "0.000000");
        assert_eq!(java_format_f(2147483647.0), "2147483647.000000");
        assert_eq!(java_format_f(-1.5), "-1.500000");
        assert_eq!(java_format_f(0.12345678), "0.123457");
        assert_eq!(java_format_f(9.9999999), "10.000000");
    }

    #[test]
    fn assignment_converter() {
        let v = ResourceConverterAssignmentConverter
            .convert(Some("memory=1.5"), None)
            .unwrap()
            .unwrap();
        assert_eq!(v.get::<(String, f64)>(), ("memory".to_string(), 1.5));
        assert_eq!(
            ResourceConverterAssignmentConverter.get_type_description(),
            "a named double, 'name=value', where value is an integer, or a keyword \
             (\"HOST_CPUS\", \"HOST_RAM\"), optionally followed by an operation ([-|*]<float>) eg. \
             \"HOST_CPUS\", \"HOST_CPUS*.5\""
        );
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/util/ResourceConverterTest.java

    use crate::misc::test_support::with_local_host_capacity;

    fn null_supplier<T>() -> Supplier<T> {
        Arc::new(|| panic!("NullPointerException: supplier returned null"))
    }

    fn int_converter(auto: Supplier<i32>, min: i32, max: i32) -> ResourceConverterIntegerConverter {
        ResourceConverterIntegerConverter::new(auto, min, max)
    }

    #[test]
    fn convert_number_returns_int() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        assert_eq!(c.convert_int("6").unwrap(), 6);
    }

    #[test]
    fn convert_number_returns_double() {
        let c = ResourceConverterDoubleConverter::new(null_supplier(), 1.0, f64::MAX);
        assert_eq!(c.convert_double("6.3").unwrap(), 6.3);
    }

    #[test]
    fn convert_number_greater_than_max_throws_exception() {
        let c = int_converter(null_supplier(), 0, 1);
        let e = c.convert_int("2").unwrap_err();
        assert!(e.get_message().contains("cannot be greater than 1"));
    }

    #[test]
    fn convert_number_less_than_min_throws_exception() {
        let c = int_converter(null_supplier(), -1, 1);
        assert_eq!(c.convert_int("0").unwrap(), 0);
        let e = c.convert_int("-2").unwrap_err();
        assert!(e.get_message().contains("must be at least -1"));
    }

    #[test]
    fn convert_auto_returns_supplied_auto_value() {
        let c = int_converter(Arc::new(|| 5), 1, i32::MAX);
        assert_eq!(c.convert_int("auto").unwrap(), 5);
    }

    #[test]
    fn convert_auto_with_operator_applies_operator_to_auto() {
        let c = int_converter(Arc::new(|| 5), 1, i32::MAX);
        assert_eq!(c.convert_int("auto-1").unwrap(), 4);
    }

    #[test]
    fn convert_auto_with_invalid_operator_throws_exception() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        let e = c.convert_int("auto/2").unwrap_err();
        assert!(e.get_message().contains("does not follow correct syntax"));
    }

    #[test]
    fn convert_auto_is_float_returns_rounded_int() {
        let c = int_converter(Arc::new(|| 5), 1, i32::MAX);
        assert_eq!(c.convert_int("auto*.51").unwrap(), 3);
    }

    #[test]
    fn convert_host_cpus_returns_cpu_setting() {
        with_local_host_capacity((1.0, 15.0), || {
            let c = int_converter(Arc::new(|| 5), 1, i32::MAX);
            assert_eq!(c.convert_int("HOST_CPUS").unwrap(), 15);
        });
    }

    #[test]
    fn convert_ram_returns_ram_setting() {
        with_local_host_capacity((10.0, 0.0), || {
            let c = int_converter(Arc::new(|| 5), 1, i32::MAX);
            assert_eq!(c.convert_int("HOST_RAM").unwrap(), 10);
        });
    }

    #[test]
    fn convert_float_throws_exception() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        let e = c.convert_int(".5").unwrap_err();
        assert!(e.get_message().contains("This flag takes an integer"));
    }

    #[test]
    fn convert_wrong_keyword_throws_exception() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        let e = c.convert_int("invalid_keyword").unwrap_err();
        assert_eq!(
            e.get_message(),
            "Parameter 'invalid_keyword' does not follow correct syntax. This flag takes an \
             integer, or a keyword (\"auto\", \"HOST_CPUS\", \"HOST_RAM\"), optionally followed \
             by an operation ([-|*]<float>) eg. \"auto\", \"HOST_CPUS*.5\"."
        );
    }

    #[test]
    fn convert_almost_valid_keyword_throws_exception() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        let e = c.convert_int("aut").unwrap_err();
        assert!(e.get_message().contains("does not follow correct syntax"));
    }

    #[test]
    fn build_converter_before_resources_uses_resources() {
        let c = int_converter(null_supplier(), 1, i32::MAX);
        with_local_host_capacity((0.0, 15.0), || {
            assert_eq!(c.convert_int("HOST_CPUS").unwrap(), 15);
        });
    }
}
