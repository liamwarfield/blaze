//! `com.github.benmanes.caffeine.cache.CaffeineSpec` (Caffeine 3.1.8, the version Bazel 9.3.2
//! pins in `MODULE.bazel`), the value of `Converters.CaffeineSpecConverter`.
//!
//! PORT: Caffeine's sources are not in the pinned tree; this is ported from the class in the
//! oracle's `A-server.jar` (`javap -c -p`), with the expected values of the tests recorded from
//! that class (`scripts/caffeine-spec/run`). `toBuilder()` (building a cache) is not ported.

use std::fmt;

use crate::value::OptionScalar;

/// `Caffeine.Strength`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Strength {
    WEAK,
    SOFT,
}

impl fmt::Display for Strength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Strength::WEAK => "WEAK",
            Strength::SOFT => "SOFT",
        })
    }
}

/// `java.time.Duration`: seconds and a nanosecond adjustment in `0..1_000_000_000`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JavaDuration {
    pub seconds: i64,
    pub nanos: i32,
}

impl JavaDuration {
    // Java (JDK): java/time/Duration.java Duration.ofSeconds(long, long)
    fn of_seconds(seconds: i64, nano_adjustment: i64) -> Option<JavaDuration> {
        let secs = seconds.checked_add(nano_adjustment.div_euclid(1_000_000_000))?;
        let nos = nano_adjustment.rem_euclid(1_000_000_000) as i32;
        Some(JavaDuration {
            seconds: secs,
            nanos: nos,
        })
    }

    // Java (JDK): java/time/Duration.java Duration.ofNanos
    fn of_nanos(nanos: i64) -> JavaDuration {
        JavaDuration::of_seconds(0, nanos).expect("no overflow")
    }

    // Java (JDK): java/time/Duration.java Duration.negated (multipliedBy(-1))
    fn negated(self) -> Option<JavaDuration> {
        // seconds * -1 with the nanos folded in, as BigDecimal arithmetic does.
        let total = (self.seconds as i128) * 1_000_000_000 + self.nanos as i128;
        let neg = -total;
        let seconds = neg.div_euclid(1_000_000_000);
        let nanos = neg.rem_euclid(1_000_000_000) as i32;
        Some(JavaDuration {
            seconds: i64::try_from(seconds).ok()?,
            nanos,
        })
    }

    // Java (JDK): java/time/Duration.java Duration.isNegative
    fn is_negative(&self) -> bool {
        self.seconds < 0
    }
}

/// A specification of a Caffeine cache builder configuration (`CaffeineSpec`).
// Java (Caffeine 3.1.8): com/github/benmanes/caffeine/cache/CaffeineSpec.java CaffeineSpec
#[derive(Clone, Debug)]
pub struct CaffeineSpec {
    specification: String,
    pub initial_capacity: i32,
    pub maximum_weight: i64,
    pub maximum_size: i64,
    pub record_stats: bool,
    pub key_strength: Option<Strength>,
    pub value_strength: Option<Strength>,
    pub expire_after_write: Option<JavaDuration>,
    pub expire_after_access: Option<JavaDuration>,
    pub refresh_after_write: Option<JavaDuration>,
}

/// `Caffeine.UNSET_INT`.
pub const UNSET_INT: i64 = -1;

/// `IllegalArgumentException(message)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IllegalArgumentException(pub String);

/// `Caffeine.requireArgument(expression, template, args)`:
/// `String.format(Locale.US, template, args)`.
fn require_argument(expression: bool, message: impl FnOnce() -> String) -> Result<(), IllegalArgumentException> {
    if !expression {
        return Err(IllegalArgumentException(message()));
    }
    Ok(())
}

/// `String.format(Locale.US, "%,d", n)`.
fn format_grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// `String.trim()`: removes the chars `<= ' '` at both ends.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// `String.split(sep)` with limit 0 for a single literal char: trailing empty strings are
/// removed; an input without the separator (also "") is returned whole.
fn java_split(input: &str, sep: char) -> Vec<&str> {
    if !input.contains(sep) {
        return vec![input];
    }
    let mut parts: Vec<&str> = input.split(sep).collect();
    while parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
}

/// `Long.parseLong(s, 10)` on the chars of `s` (`Character.digit` accepts only ASCII digits for
/// the chars an internal string holds; the bounds are those of a `long`).
fn java_parse_long(s: &str) -> Option<i64> {
    let (negative, digits) = match s.chars().next()? {
        '-' => (true, &s[1..]),
        '+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut result: i64 = 0;
    for c in digits.chars() {
        let d = c as i64 - '0' as i64;
        result = result.checked_mul(10)?;
        result = if negative {
            result.checked_sub(d)?
        } else {
            result.checked_add(d)?
        };
    }
    Some(result)
}

/// `Integer.parseInt(s, 10)`.
fn java_parse_int(s: &str) -> Option<i32> {
    java_parse_long(s).and_then(|v| i32::try_from(v).ok())
}

/// `java.time.Duration.parse(text)`. A failure is a `DateTimeParseException`, which is not an
/// `IllegalArgumentException`: neither `CaffeineSpec` nor `CaffeineSpecConverter` catches it.
// Java (JDK): java/time/Duration.java Duration.parse
fn duration_parse(text: &str) -> JavaDuration {
    // PATTERN = "([-+]?)P(?:([-+]?[0-9]+)D)?(T(?:([-+]?[0-9]+)H)?(?:([-+]?[0-9]+)M)?
    //            (?:([-+]?[0-9]+)(?:[.,]([0-9]{0,9}))?S)?)?", CASE_INSENSITIVE
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            "^(?i:([-+]?)P(?:([-+]?[0-9]+)D)?(T(?:([-+]?[0-9]+)H)?(?:([-+]?[0-9]+)M)?\
             (?:([-+]?[0-9]+)(?:[.,]([0-9]{0,9}))?S)?)?)$",
        )
        .unwrap()
    });
    let fail = |detail: &str| -> ! {
        panic!("DateTimeParseException: Text cannot be parsed to a Duration{detail}")
    };
    // Java's CASE_INSENSITIVE folds ASCII only (no UNICODE_CASE); the regex crate's `(?i)` also
    // matches 'ſ' (U+017F) for 's', a char an internal string (chars <= U+00FF) cannot hold.
    let Some(m) = PATTERN.captures(text) else {
        fail("")
    };
    // check for letter T but no time sections
    if m.get(3).is_some_and(|t| t.as_str().eq_ignore_ascii_case("T")) {
        fail("");
    }
    let negate = m.get(1).is_some_and(|g| g.as_str() == "-");
    let (day, hour, minute, second, fraction) = (m.get(2), m.get(4), m.get(5), m.get(6), m.get(7));
    if day.is_none() && hour.is_none() && minute.is_none() && second.is_none() {
        fail("");
    }
    let parse_number = |g: Option<regex::Match<'_>>, multiplier: i64, error_text: &str| -> i64 {
        let Some(g) = g else { return 0 };
        java_parse_long(g.as_str())
            .and_then(|v| v.checked_mul(multiplier))
            .unwrap_or_else(|| fail(&format!(": {error_text}")))
    };
    let days_as_secs = parse_number(day, 86_400, "days");
    let hours_as_secs = parse_number(hour, 3_600, "hours");
    let mins_as_secs = parse_number(minute, 60, "minutes");
    let seconds = parse_number(second, 1, "seconds");
    let negative_secs = second.is_some_and(|g| g.as_str().starts_with('-'));
    let nanos: i64 = match fraction {
        Some(f) if !f.as_str().is_empty() => {
            let mut fraction = java_parse_int(f.as_str()).expect("[0-9]{0,9}") as i64;
            for _ in f.as_str().len()..9 {
                fraction *= 10;
            }
            fraction * if negative_secs { -1 } else { 1 }
        }
        _ => 0,
    };
    let total = days_as_secs
        .checked_add(hours_as_secs)
        .and_then(|s| s.checked_add(mins_as_secs))
        .and_then(|s| s.checked_add(seconds));
    let duration = total
        .and_then(|s| JavaDuration::of_seconds(s, nanos))
        .and_then(|d| if negate { d.negated() } else { Some(d) });
    duration.unwrap_or_else(|| fail(": overflow"))
}

/// `TimeUnit.toNanos(d)`: saturating.
fn to_nanos(unit_nanos: i64, d: i64) -> i64 {
    d.saturating_mul(unit_nanos)
}

impl CaffeineSpec {
    // Java (Caffeine 3.1.8): CaffeineSpec.<init>
    fn new(specification: &str) -> CaffeineSpec {
        CaffeineSpec {
            specification: specification.to_string(),
            initial_capacity: UNSET_INT as i32,
            maximum_weight: UNSET_INT,
            maximum_size: UNSET_INT,
            record_stats: false,
            key_strength: None,
            value_strength: None,
            expire_after_write: None,
            expire_after_access: None,
            refresh_after_write: None,
        }
    }

    /// Creates a CaffeineSpec from a string (comma-separated `key=value` options).
    // Java (Caffeine 3.1.8): CaffeineSpec.parse
    pub fn parse(specification: &str) -> Result<CaffeineSpec, IllegalArgumentException> {
        let mut spec = CaffeineSpec::new(specification);
        for option in java_split(specification, ',') {
            spec.parse_option(java_trim(option))?;
        }
        Ok(spec)
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.parseOption
    fn parse_option(&mut self, option: &str) -> Result<(), IllegalArgumentException> {
        if option.is_empty() {
            return Ok(());
        }
        let key_and_value = java_split(option, '=');
        require_argument(key_and_value.len() <= 2, || {
            format!("key-value pair {option} with more than one equals sign")
        })?;
        let key = java_trim(key_and_value[0]);
        let value = if key_and_value.len() == 1 {
            None
        } else {
            Some(java_trim(key_and_value[1]))
        };
        self.configure(key, value)
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.configure
    fn configure(&mut self, key: &str, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        match key {
            "initialCapacity" => self.initial_capacity(key, value),
            "maximumSize" => self.maximum_size(key, value),
            "maximumWeight" => self.maximum_weight(key, value),
            "weakKeys" => self.weak_keys(value),
            "weakValues" => self.value_strength(key, value, Strength::WEAK),
            "softValues" => self.value_strength(key, value, Strength::SOFT),
            "expireAfterAccess" => {
                require_argument(self.expire_after_access.is_none(), || {
                    "expireAfterAccess was already set".to_string()
                })?;
                self.expire_after_access = Some(parse_duration(key, value)?);
                Ok(())
            }
            "expireAfterWrite" => {
                require_argument(self.expire_after_write.is_none(), || {
                    "expireAfterWrite was already set".to_string()
                })?;
                self.expire_after_write = Some(parse_duration(key, value)?);
                Ok(())
            }
            "refreshAfterWrite" => {
                require_argument(self.refresh_after_write.is_none(), || {
                    "refreshAfterWrite was already set".to_string()
                })?;
                self.refresh_after_write = Some(parse_duration(key, value)?);
                Ok(())
            }
            "recordStats" => self.record_stats(value),
            _ => Err(IllegalArgumentException(format!("Unknown key {key}"))),
        }
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.initialCapacity
    fn initial_capacity(&mut self, key: &str, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        require_argument(self.initial_capacity == UNSET_INT as i32, || {
            format!(
                "initial capacity was already set to {}",
                format_grouped(self.initial_capacity as i64)
            )
        })?;
        self.initial_capacity = parse_int(key, value)?;
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.maximumSize
    fn maximum_size(&mut self, key: &str, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        require_argument(self.maximum_size == UNSET_INT, || {
            format!(
                "maximum size was already set to {}",
                format_grouped(self.maximum_size)
            )
        })?;
        require_argument(self.maximum_weight == UNSET_INT, || {
            format!(
                "maximum weight was already set to {}",
                format_grouped(self.maximum_weight)
            )
        })?;
        self.maximum_size = parse_long(key, value)?;
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.maximumWeight
    fn maximum_weight(&mut self, key: &str, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        require_argument(self.maximum_weight == UNSET_INT, || {
            format!(
                "maximum weight was already set to {}",
                format_grouped(self.maximum_weight)
            )
        })?;
        require_argument(self.maximum_size == UNSET_INT, || {
            format!(
                "maximum size was already set to {}",
                format_grouped(self.maximum_size)
            )
        })?;
        self.maximum_weight = parse_long(key, value)?;
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.weakKeys
    fn weak_keys(&mut self, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        require_argument(value.is_none(), || {
            "weak keys does not take a value".to_string()
        })?;
        require_argument(self.key_strength.is_none(), || {
            "weak keys was already set".to_string()
        })?;
        self.key_strength = Some(Strength::WEAK);
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.valueStrength
    fn value_strength(
        &mut self,
        key: &str,
        value: Option<&str>,
        strength: Strength,
    ) -> Result<(), IllegalArgumentException> {
        require_argument(value.is_none(), || format!("{key} does not take a value"))?;
        require_argument(self.value_strength.is_none(), || {
            format!(
                "{key} was already set to {}",
                self.value_strength.expect("set")
            )
        })?;
        self.value_strength = Some(strength);
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.recordStats
    fn record_stats(&mut self, value: Option<&str>) -> Result<(), IllegalArgumentException> {
        require_argument(value.is_none(), || {
            "record stats does not take a value".to_string()
        })?;
        require_argument(!self.record_stats, || {
            "record stats was already set".to_string()
        })?;
        self.record_stats = true;
        Ok(())
    }

    // Java (Caffeine 3.1.8): CaffeineSpec.toParsableString
    pub fn to_parsable_string(&self) -> &str {
        &self.specification
    }
}

// Java (Caffeine 3.1.8): CaffeineSpec.parseInt
fn parse_int(key: &str, value: Option<&str>) -> Result<i32, IllegalArgumentException> {
    let value = value.filter(|v| !v.is_empty());
    require_argument(value.is_some(), || format!("value of key {key} was omitted"))?;
    let value = value.unwrap();
    java_parse_int(value).ok_or_else(|| {
        IllegalArgumentException(format!(
            "key {key} value was set to {value}, must be an integer"
        ))
    })
}

// Java (Caffeine 3.1.8): CaffeineSpec.parseLong
fn parse_long(key: &str, value: Option<&str>) -> Result<i64, IllegalArgumentException> {
    let value = value.filter(|v| !v.is_empty());
    require_argument(value.is_some(), || format!("value of key {key} was omitted"))?;
    let value = value.unwrap();
    java_parse_long(value).ok_or_else(|| {
        IllegalArgumentException(format!("key {key} value was set to {value}, must be a long"))
    })
}

// Java (Caffeine 3.1.8): CaffeineSpec.parseDuration
fn parse_duration(key: &str, value: Option<&str>) -> Result<JavaDuration, IllegalArgumentException> {
    let value = value.filter(|v| !v.is_empty());
    require_argument(value.is_some(), || format!("value of key {key} omitted"))?;
    let value = value.unwrap();
    let iso_format = value.contains('p') || value.contains('P');
    if iso_format {
        let duration = duration_parse(value);
        require_argument(!duration.is_negative(), || {
            format!("key {key} invalid format; was {value}, but the duration cannot be negative")
        })?;
        return Ok(duration);
    }
    // `value.substring(0, value.length() - 1)`: Java counts UTF-16 units; the last char of an
    // internal string is one unit.
    let last = value.chars().last().expect("non-empty");
    let duration = parse_long(key, Some(&value[..value.len() - last.len_utf8()]))?;
    let unit = parse_time_unit(key, Some(value))?;
    Ok(JavaDuration::of_nanos(to_nanos(unit, duration)))
}

/// `parseTimeUnit`: the unit's length in nanoseconds.
// Java (Caffeine 3.1.8): CaffeineSpec.parseTimeUnit
fn parse_time_unit(key: &str, value: Option<&str>) -> Result<i64, IllegalArgumentException> {
    let value = value.filter(|v| !v.is_empty());
    require_argument(value.is_some(), || format!("value of key {key} omitted"))?;
    let value = value.unwrap();
    let last = value.chars().last().expect("non-empty");
    // `Character.toLowerCase(char)`; only the ASCII letters d, h, m, s are units, and no other
    // char <= U+00FF lowercases to one of them.
    match last.to_ascii_lowercase() {
        'd' => Ok(86_400_000_000_000),
        'h' => Ok(3_600_000_000_000),
        'm' => Ok(60_000_000_000),
        's' => Ok(1_000_000_000),
        _ => Err(IllegalArgumentException(format!(
            "key {key} invalid format; was {value}, must end with one of [dDhHmMsS]"
        ))),
    }
}

impl PartialEq for CaffeineSpec {
    // Java (Caffeine 3.1.8): CaffeineSpec.equals (the specification string is not compared)
    fn eq(&self, o: &CaffeineSpec) -> bool {
        self.refresh_after_write == o.refresh_after_write
            && self.expire_after_access == o.expire_after_access
            && self.expire_after_write == o.expire_after_write
            && self.initial_capacity == o.initial_capacity
            && self.maximum_weight == o.maximum_weight
            && self.value_strength == o.value_strength
            && self.key_strength == o.key_strength
            && self.maximum_size == o.maximum_size
            && self.record_stats == o.record_stats
    }
}

impl fmt::Display for CaffeineSpec {
    // Java (Caffeine 3.1.8): CaffeineSpec.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CaffeineSpec{{{}}}", self.to_parsable_string())
    }
}

impl OptionScalar for CaffeineSpec {
    fn java_to_string(&self) -> String {
        self.to_string()
    }
}

crate::option_field_type!(
    CaffeineSpec,
    "com.github.benmanes.caffeine.cache.CaffeineSpec",
    raw = CaffeineSpec::new("")
);
