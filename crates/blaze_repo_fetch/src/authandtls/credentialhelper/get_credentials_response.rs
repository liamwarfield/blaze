//! Response from the `get` command of the Credential Helper Protocol.

use super::gson::{self, JsonSyntaxException, JsonValue};
use crate::authandtls::Headers;

/// Why `GSON.fromJson(json, GetCredentialsResponse.class)` failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseParseError {
    /// `JsonSyntaxException`.
    JsonSyntax(JsonSyntaxException),
    /// `IllegalArgumentException` (a repeated header name), with its message; Gson does not wrap
    /// it.
    IllegalArgument(String),
}

impl std::fmt::Display for ResponseParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponseParseError::JsonSyntax(e) => e.fmt(f),
            ResponseParseError::IllegalArgument(message) => f.write_str(message),
        }
    }
}

/// `List.toString()`: `[a, b]`.
fn java_list_to_string(values: &[String]) -> String {
    format!("[{}]", values.join(", "))
}

/// `java.time.Instant`: seconds since the epoch and a nanosecond adjustment in `0..1e9`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaInstant {
    pub seconds: i64,
    pub nanos: i32,
}

impl JavaInstant {
    /// `Instant.ofEpochSecond(seconds)`.
    pub fn of_epoch_second(seconds: i64) -> JavaInstant {
        JavaInstant { seconds, nanos: 0 }
    }

    /// `Instant.ofEpochMilli(millis)`.
    pub fn of_epoch_milli(millis: i64) -> JavaInstant {
        JavaInstant {
            seconds: millis.div_euclid(1000),
            nanos: (millis.rem_euclid(1000) * 1_000_000) as i32,
        }
    }

    /// `Instant.ofEpochSecond(0, nanos)`.
    pub fn of_epoch_nano(nanos: i64) -> JavaInstant {
        JavaInstant {
            seconds: nanos.div_euclid(1_000_000_000),
            nanos: nanos.rem_euclid(1_000_000_000) as i32,
        }
    }

    /// `instant.plus(duration)` for a non-negative `java.time.Duration`.
    pub fn plus(self, duration: std::time::Duration) -> JavaInstant {
        let nanos = self.nanos as i64 + duration.subsec_nanos() as i64;
        JavaInstant {
            seconds: self.seconds + duration.as_secs() as i64 + nanos / 1_000_000_000,
            nanos: (nanos % 1_000_000_000) as i32,
        }
    }

    /// The nanoseconds since the epoch, as `i128` (Java computes them with `Duration`).
    pub fn epoch_nanos(self) -> i128 {
        self.seconds as i128 * 1_000_000_000 + self.nanos as i128
    }
}

/// Response from the `get` command of the Credential Helper Protocol
/// (<https://github.com/EngFlow/credential-helper-spec/blob/main/schemas/get-credentials-response.schema.json>).
///
/// `headers`: the headers to attach to the request. `expires`: the time the credentials expire
/// and must be revalidated.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:47 GetCredentialsResponse
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GetCredentialsResponse {
    headers: Headers,
    expires: Option<JavaInstant>,
}

impl GetCredentialsResponse {
    /// `GetCredentialsResponse.newBuilder()...build()`.
    ///
    /// Panics as `ImmutableMap.Builder.buildOrThrow` does on a repeated header name (the builder
    /// cannot express one here: `headers` is a map).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:62 GetCredentialsResponse.newBuilder
    pub fn new(headers: Headers, expires: Option<JavaInstant>) -> GetCredentialsResponse {
        GetCredentialsResponse { headers, expires }
    }

    /// `headers()`.
    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    /// `expires()`.
    pub fn expires(&self) -> Option<JavaInstant> {
        self.expires
    }

    /// `GSON.toJson(response)` (`GsonTypeAdapter.write`).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:79 GetCredentialsResponse.GsonTypeAdapter.write
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        let mut first = true;
        if !self.headers.is_empty() {
            gson::write_string(&mut out, "headers");
            out.push(':');
            out.push('{');
            for (i, (name, values)) in self.headers.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                gson::write_string(&mut out, name);
                out.push(':');
                out.push('[');
                for (j, value) in values.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    gson::write_string(&mut out, value);
                }
                out.push(']');
            }
            out.push('}');
            first = false;
        }
        if let Some(expires) = self.expires {
            if !first {
                out.push(',');
            }
            gson::write_string(&mut out, "expires");
            out.push(':');
            gson::write_string(&mut out, &rfc_3339_format(expires));
        }
        out.push('}');
        out
    }

    /// `GSON.fromJson(json, GetCredentialsResponse.class)`: `Ok(None)` is Java's `null` (no
    /// output).
    // Java: com/google/gson/Gson.java Gson.fromJson
    pub fn parse(json: &str) -> Result<Option<GetCredentialsResponse>, ResponseParseError> {
        match gson::from_json(json).map_err(ResponseParseError::JsonSyntax)? {
            None => Ok(None),
            Some(value) => GetCredentialsResponse::from_json(&value).map(Some),
        }
    }

    /// `GsonTypeAdapter.read`, over the parsed document.
    ///
    /// The header names of every `"headers"` member go into one `ImmutableMap.Builder`, whose
    /// `build()` throws `IllegalArgumentException` on a repeated name.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:110 GetCredentialsResponse.GsonTypeAdapter.read
    fn from_json(value: &JsonValue) -> Result<GetCredentialsResponse, ResponseParseError> {
        // response.headersBuilder()
        let mut headers_builder: Vec<(String, Vec<String>)> = Vec::new();
        let mut expires = None;

        let JsonValue::Object(members) = value else {
            return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                format!("Expected object, got {}", value.token_name()),
            )));
        };

        for (name, value) in members {
            match name.as_str() {
                "headers" => {
                    let JsonValue::Object(header_members) = value else {
                        return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                            format!(
                                "Expected value of 'headers' to be an object, got {}",
                                value.token_name()
                            ),
                        )));
                    };

                    for (header_name, header_value) in header_members {
                        let mut header_values = Vec::new();

                        let JsonValue::Array(elements) = header_value else {
                            return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                                format!(
                                    "Expected value of '{}' header to be an array of strings, got {}",
                                    header_name,
                                    header_value.token_name()
                                ),
                            )));
                        };
                        for (i, element) in elements.iter().enumerate() {
                            let JsonValue::String(s) = element else {
                                return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                                    format!(
                                        "Expected value {} of '{}' header to be a string, got {}",
                                        i,
                                        header_name,
                                        element.token_name()
                                    ),
                                )));
                            };
                            header_values.push(s.clone());
                        }

                        headers_builder.push((header_name.clone(), header_values));
                    }
                }
                "expires" => {
                    let JsonValue::String(s) = value else {
                        return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                            format!(
                                "Expected value of 'expires' to be a string, got {}",
                                value.token_name()
                            ),
                        )));
                    };
                    match rfc_3339_parse(s) {
                        Ok(instant) => expires = Some(instant),
                        Err(message) => {
                            return Err(ResponseParseError::JsonSyntax(JsonSyntaxException(
                                format!(
                                    "Expected value of 'expires' to be a RFC 3339 formatted timestamp: {message}"
                                ),
                            )));
                        }
                    }
                }
                // We intentionally ignore unknown keys to achieve forward compatibility with
                // responses coming from newer tools.
                _ => {}
            }
        }
        // response.build(): ImmutableMap.Builder.buildOrThrow.
        let mut headers = Headers::new();
        for (name, values) in headers_builder {
            if let Some(earlier) = headers.get(&name) {
                // Guava: conflictException("key", entry, earlierEntry).
                return Err(ResponseParseError::IllegalArgument(format!(
                    "Multiple entries with same key: {}={} and {}={}",
                    name,
                    java_list_to_string(&values),
                    name,
                    java_list_to_string(earlier)
                )));
            }
            headers.insert(name, values);
        }
        Ok(GetCredentialsResponse { headers, expires })
    }
}

/// `LocalDate.toEpochDay` of a proleptic Gregorian date (`month` 1..=12, `day` valid).
fn epoch_day(year: i64, month: i64, day: i64) -> i64 {
    // Java: java/time/LocalDate.java LocalDate.toEpochDay
    let y = year;
    let m = month;
    let mut total = 365 * y;
    if y >= 0 {
        total += (y + 3) / 4 - (y + 99) / 100 + (y + 399) / 400;
    } else {
        total -= y / -4 - y / -100 + y / -400;
    }
    total += (367 * m - 362) / 12;
    total += day - 1;
    if m > 2 {
        total -= 1;
        if !is_leap(y) {
            total -= 1;
        }
    }
    total - DAYS_0000_TO_1970
}

/// `LocalDate.DAYS_0000_TO_1970`.
const DAYS_0000_TO_1970: i64 = (146097 * 5) - (30 * 365 + 7);

fn is_leap(year: i64) -> bool {
    (year & 3) == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// `LocalDate.ofEpochDay(epochDay)` as (year, month, day).
// Java: java/time/LocalDate.java LocalDate.ofEpochDay
fn civil_from_epoch_day(epoch_day: i64) -> (i64, i64, i64) {
    let mut zero_day = epoch_day + DAYS_0000_TO_1970;
    // find the march-based year
    zero_day -= 60; // adjust to 0000-03-01 so leap day is at end of four year cycle
    let mut adjust = 0;
    if zero_day < 0 {
        // adjust negative years to positive for calculation
        let adjust_cycles = (zero_day + 1) / 146097 - 1;
        adjust = adjust_cycles * 400;
        zero_day += -adjust_cycles * 146097;
    }
    let mut year_est = (400 * zero_day + 591) / 146097;
    let mut doy_est = zero_day - (365 * year_est + year_est / 4 - year_est / 100 + year_est / 400);
    if doy_est < 0 {
        // fix estimate
        year_est -= 1;
        doy_est = zero_day - (365 * year_est + year_est / 4 - year_est / 100 + year_est / 400);
    }
    year_est += adjust; // reset any negative year
    let march_doy0 = doy_est;

    // convert march-based values back to january-based
    let march_month0 = (march_doy0 * 5 + 2) / 153;
    let month = (march_month0 + 2) % 12 + 1;
    let dom = march_doy0 - (march_month0 * 306 + 5) / 10 + 1;
    year_est += march_month0 / 10;
    (year_est, month, dom)
}

/// `LocalDate.MIN`/`MAX` years (`Year.MIN_VALUE`, `Year.MAX_VALUE`).
const YEAR_MIN: i64 = -999_999_999;
const YEAR_MAX: i64 = 999_999_999;

/// `LocalDate.of(year, 1, 1).plusMonths(months).plusDays(days)` as an epoch day (the
/// `ResolverStyle.LENIENT` resolution of year, month and day in `IsoChronology.resolveYMD`).
fn lenient_epoch_day(year: i64, months: i64, days: i64) -> Result<i64, String> {
    if !(YEAR_MIN..=YEAR_MAX).contains(&year) {
        return Err(format!(
            "Invalid value for Year (valid values -999999999 - 999999999): {year}"
        ));
    }
    // LocalDate.plusMonths
    let month_count = year * 12;
    let calc_months = month_count + months;
    let new_year = calc_months.div_euclid(12);
    if !(YEAR_MIN..=YEAR_MAX).contains(&new_year) {
        return Err(format!(
            "Invalid value for Year (valid values -999999999 - 999999999): {new_year}"
        ));
    }
    let new_month = calc_months.rem_euclid(12) + 1;
    // The day of month is 1, which every month has.
    Ok(epoch_day(new_year, new_month, 1) + days)
}

/// Parses one fixed-width group of ASCII digits (`appendValue(field, width)`, strict parsing).
fn fixed_digits(text: &[u8], pos: &mut usize, width: usize) -> Option<i64> {
    let digits = text.get(*pos..*pos + width)?;
    let mut value = 0i64;
    for &d in digits {
        if !d.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (d - b'0') as i64;
    }
    *pos += width;
    Some(value)
}

fn literal(text: &[u8], pos: &mut usize, c: u8) -> Option<()> {
    if text.get(*pos) == Some(&c) {
        *pos += 1;
        Some(())
    } else {
        None
    }
}

/// `Instant.from(RFC_3339_FORMATTER.parse(text))`: `DateTimeFormatter.ofPattern(
/// "yyyy-MM-dd'T'HH:mm:ssXXX").withZone(UTC).withResolverStyle(LENIENT)`. `Err` is a
/// `DateTimeException`'s message.
///
/// The fields are parsed strictly (fixed widths, the year as `SignStyle.EXCEEDS_PAD` with 4 to
/// 19 digits) and resolved leniently: month, day, hour, minute and second roll over into the
/// next larger unit. The offset (`+HH:MM` or `Z`) takes precedence over the formatter's zone.
///
/// PORT: the `DateTimeException` messages are not the JDK's (the caller prints none of them).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:56 GetCredentialsResponse.RFC_3339_FORMATTER
pub(crate) fn rfc_3339_parse(s: &str) -> Result<JavaInstant, String> {
    let unparseable = || format!("Text '{s}' could not be parsed");
    let text = s.as_bytes();
    let mut pos = 0usize;

    // yyyy: YEAR_OF_ERA, min width 4, max width 19, SignStyle.EXCEEDS_PAD, strict.
    let (negative, positive) = match text.first() {
        Some(b'+') => (false, true),
        Some(b'-') => (true, false),
        _ => (false, false),
    };
    if negative || positive {
        pos += 1;
    }
    let digits_start = pos;
    while pos < text.len() && pos - digits_start < 19 && text[pos].is_ascii_digit() {
        pos += 1;
    }
    let parse_len = pos - digits_start;
    if parse_len < 4 {
        return Err(unparseable());
    }
    if positive && parse_len <= 4 {
        return Err(unparseable());
    }
    if !negative && !positive && parse_len > 4 {
        return Err(unparseable());
    }
    let digits = &s[digits_start..pos];
    // More than 18 digits is out of every year range; Java resolves it to the same error.
    let mut year_of_era: i64 = if parse_len > 18 {
        return Err(unparseable());
    } else {
        digits.parse::<i64>().map_err(|_| unparseable())?
    };
    if negative {
        if year_of_era == 0 {
            return Err(unparseable());
        }
        year_of_era = -year_of_era;
    }

    literal(text, &mut pos, b'-').ok_or_else(unparseable)?;
    let month = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
    literal(text, &mut pos, b'-').ok_or_else(unparseable)?;
    let day = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
    literal(text, &mut pos, b'T').ok_or_else(unparseable)?;
    let hour = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
    literal(text, &mut pos, b':').ok_or_else(unparseable)?;
    let minute = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
    literal(text, &mut pos, b':').ok_or_else(unparseable)?;
    let second = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;

    // XXX: appendOffset("+HH:MM", "Z").
    let offset_seconds: i64 = match text.get(pos) {
        Some(b'Z') => {
            pos += 1;
            0
        }
        Some(&sign @ (b'+' | b'-')) => {
            pos += 1;
            let hours = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
            literal(text, &mut pos, b':').ok_or_else(unparseable)?;
            let minutes = fixed_digits(text, &mut pos, 2).ok_or_else(unparseable)?;
            if hours > 23 || minutes > 59 {
                return Err(
                    "Value out of range: Hour[0-23], Minute[0-59], Second[0-59]".to_string()
                );
            }
            let secs = hours * 3600 + minutes * 60;
            if sign == b'-' { -secs } else { secs }
        }
        _ => return Err(unparseable()),
    };
    if pos != text.len() {
        return Err(format!(
            "Text '{s}' could not be parsed, unparsed text found at index {pos}"
        ));
    }
    // ZoneOffset.ofTotalSeconds: OFFSET_SECONDS.checkValidIntValue.
    if !(-18 * 3600..=18 * 3600).contains(&offset_seconds) {
        return Err(format!(
            "Invalid value for OffsetSeconds (valid values -64800 - 64800): {offset_seconds}"
        ));
    }

    // IsoChronology.resolveYearOfEra (no era, lenient): YEAR = YEAR_OF_ERA.
    // IsoChronology.resolveYMD (lenient).
    let epoch_day = lenient_epoch_day(year_of_era, month - 1, day - 1)?;
    // Parsed.resolveTimeLenient: the excess days of the time join the date.
    let total_seconds = hour * 3600 + minute * 60 + second;
    let excess_days = total_seconds.div_euclid(86400);
    let second_of_day = total_seconds.rem_euclid(86400);
    let epoch_day = epoch_day + excess_days;
    let (year, _, _) = civil_from_epoch_day(epoch_day);
    if !(YEAR_MIN..=YEAR_MAX).contains(&year) {
        return Err(format!(
            "Invalid value for Year (valid values -999999999 - 999999999): {year}"
        ));
    }
    Ok(JavaInstant::of_epoch_second(
        epoch_day * 86400 + second_of_day - offset_seconds,
    ))
}

/// `RFC_3339_FORMATTER.format(instant)`: the instant in UTC, without fractional seconds, with
/// the offset printed as `Z`.
///
/// PORT: years outside 0..=9999 (printed by `yyyy` with a sign or as year-of-era) are not
/// ported.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/GetCredentialsResponse.java:56 GetCredentialsResponse.RFC_3339_FORMATTER
pub(crate) fn rfc_3339_format(instant: JavaInstant) -> String {
    let epoch_day = instant.seconds.div_euclid(86400);
    let second_of_day = instant.seconds.rem_euclid(86400);
    let (year, month, day) = civil_from_epoch_day(epoch_day);
    if !(1..=9999).contains(&year) {
        blaze_util::unported!(
            "GetCredentialsResponse.RFC_3339_FORMATTER.format (year outside 1..9999)"
        );
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        second_of_day / 3600,
        second_of_day / 60 % 60,
        second_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_day_round_trips() {
        for day in [-800_000i64, -1, 0, 1, 59, 60, 365, 11_016, 2_932_896] {
            let (y, m, d) = civil_from_epoch_day(day);
            assert_eq!(epoch_day(y, m, d), day, "{y}-{m}-{d}");
        }
        assert_eq!(civil_from_epoch_day(0), (1970, 1, 1));
    }

    #[test]
    fn rejects_text_outside_the_pattern() {
        assert!(rfc_3339_parse("1970-01-01T00:00:00.5Z").is_err());
        assert!(rfc_3339_parse("70-01-01T00:00:00Z").is_err());
        assert!(rfc_3339_parse("1970-01-01T00:00:00").is_err());
        assert!(rfc_3339_parse("1970-01-01T00:00:00+0000").is_err());
        assert!(rfc_3339_parse("1970-01-01T00:00:00Zx").is_err());
    }
}
