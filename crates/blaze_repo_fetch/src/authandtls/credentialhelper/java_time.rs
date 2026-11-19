//! The parts of `java.time` the credential helper protocol uses: `Instant`, and the formatter
//! `GetCredentialsResponse.RFC_3339_FORMATTER`:
//!
//! ```java
//! DateTimeFormatter.ofPattern("yyyy-MM-dd'T'HH:mm:ssXXX")
//!     .withZone(ZoneId.from(ZoneOffset.UTC))
//!     .withResolverStyle(ResolverStyle.LENIENT);
//! ```
//!
//! PORT: the JDK is third-party; this ports the behaviour of that one formatter (strict parsing,
//! lenient resolving, ISO proleptic calendar), not `java.time` itself.

use std::fmt;
use std::time::Duration;

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// `java.time.Instant`: a point on the time line, in nanoseconds since 1970-01-01T00:00:00Z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    epoch_nanos: i128,
}

impl Instant {
    /// `Instant.ofEpochSecond(epochSecond)`.
    pub fn of_epoch_second(epoch_second: i64) -> Instant {
        Instant {
            epoch_nanos: epoch_second as i128 * NANOS_PER_SECOND,
        }
    }

    /// `Instant.ofEpochSecond(epochSecond, nanoAdjustment)`.
    pub fn of_epoch_second_and_nanos(epoch_second: i64, nano_adjustment: i64) -> Instant {
        Instant {
            epoch_nanos: epoch_second as i128 * NANOS_PER_SECOND + nano_adjustment as i128,
        }
    }

    /// `Instant.ofEpochMilli(epochMilli)`.
    pub fn of_epoch_milli(epoch_milli: i64) -> Instant {
        Instant {
            epoch_nanos: epoch_milli as i128 * 1_000_000,
        }
    }

    /// Nanoseconds since the epoch (what `Duration.between(EPOCH, this)` holds).
    pub fn epoch_nanos(&self) -> i128 {
        self.epoch_nanos
    }

    /// `Instant.getEpochSecond()`.
    pub fn get_epoch_second(&self) -> i64 {
        self.epoch_nanos.div_euclid(NANOS_PER_SECOND) as i64
    }

    /// `Instant.plus(Duration)`.
    pub fn plus(&self, duration: Duration) -> Instant {
        Instant {
            epoch_nanos: self.epoch_nanos + duration.as_nanos() as i128,
        }
    }

    /// `Instant.minus(Duration)`.
    pub fn minus(&self, duration: Duration) -> Instant {
        Instant {
            epoch_nanos: self.epoch_nanos - duration.as_nanos() as i128,
        }
    }
}

/// `java.time.DateTimeException` (and its subclass `DateTimeParseException`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateTimeException {
    pub message: String,
}

impl fmt::Display for DateTimeException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (`LocalDate.toEpochDay`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01 (`LocalDate.ofEpochDay`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// `RFC_3339_FORMATTER.format(instant)`: `yyyy-MM-dd'T'HH:mm:ss` in UTC followed by `Z` (the
/// offset of the formatter's zone, UTC, prints as the `XXX` no-offset text). Fractions of a
/// second are not printed.
pub fn rfc_3339_format(instant: &Instant) -> String {
    let epoch_second = instant.get_epoch_second();
    let days = epoch_second.div_euclid(SECONDS_PER_DAY);
    let second_of_day = epoch_second.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    // 'yyyy' is the year-of-era, printed with SignStyle.EXCEEDS_PAD and a minimum width of 4.
    let year_of_era = if year > 0 { year } else { 1 - year };
    let year_text = if year_of_era > 9999 {
        format!("+{year_of_era}")
    } else {
        format!("{year_of_era:04}")
    };
    format!(
        "{year_text}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second_of_day / 3600,
        (second_of_day / 60) % 60,
        second_of_day % 60
    )
}

fn parse_error(text: &str, index: usize) -> DateTimeException {
    DateTimeException {
        message: format!("Text '{text}' could not be parsed at index {index}"),
    }
}

/// `Instant.from(RFC_3339_FORMATTER.parse(text))`.
///
/// The pattern is parsed strictly: `yyyy` is four digits, or a `+` followed by five to
/// nineteen digits, or a `-` followed by at least four; `MM`, `dd`, `HH`, `mm` and `ss` are exactly
/// two digits; `XXX` is `Z` or `+HH:MM` / `-HH:MM`. The fields are then resolved leniently: a
/// month or day out of range rolls over (`LocalDate.of(y, 1, 1).plusMonths(M - 1).plusDays(d -
/// 1)`), and so does a time of day (`24:00:00` is midnight of the next day).
///
/// PORT: the texts of `DateTimeException`s raised while resolving are approximations; the parse
/// errors ("could not be parsed at index N", "unparsed text found") are the JDK's.
pub fn rfc_3339_parse(text: &str) -> Result<Instant, DateTimeException> {
    let b = text.as_bytes();
    let mut pos = 0usize;

    // Exactly `width` ASCII digits at `pos`.
    let fixed = |pos: &mut usize, width: usize| -> Result<i64, DateTimeException> {
        let digits = b
            .get(*pos..*pos + width)
            .filter(|d| d.iter().all(u8::is_ascii_digit))
            .ok_or_else(|| parse_error(text, *pos))?;
        *pos += width;
        Ok(digits
            .iter()
            .fold(0i64, |acc, d| acc * 10 + (d - b'0') as i64))
    };
    let literal = |pos: &mut usize, c: u8| -> Result<(), DateTimeException> {
        if b.get(*pos) != Some(&c) {
            return Err(parse_error(text, *pos));
        }
        *pos += 1;
        Ok(())
    };

    // yyyy: appendValue(YEAR_OF_ERA, 4, 19, SignStyle.EXCEEDS_PAD), parsed strictly.
    let sign = match b.first() {
        Some(b'+') => {
            pos += 1;
            1
        }
        Some(b'-') => {
            pos += 1;
            -1
        }
        _ => 0,
    };
    let digits_start = pos;
    while pos < b.len() && pos - digits_start < 19 && b[pos].is_ascii_digit() {
        pos += 1;
    }
    let parse_len = pos - digits_start;
    if parse_len < 4 {
        return Err(parse_error(text, digits_start));
    }
    let magnitude: i128 = text[digits_start..pos].parse().unwrap();
    let year_of_era: i128 = match sign {
        -1 => {
            if magnitude == 0 {
                // minus zero not allowed
                return Err(parse_error(text, digits_start - 1));
            }
            -magnitude
        }
        1 => {
            // '+' only parsed if minWidth exceeded
            if parse_len <= 4 {
                return Err(parse_error(text, digits_start - 1));
            }
            magnitude
        }
        _ => {
            // '+' must be parsed if minWidth exceeded
            if parse_len > 4 {
                return Err(parse_error(text, digits_start));
            }
            magnitude
        }
    };
    literal(&mut pos, b'-')?;
    let month = fixed(&mut pos, 2)?;
    literal(&mut pos, b'-')?;
    let day = fixed(&mut pos, 2)?;
    literal(&mut pos, b'T')?;
    let hour = fixed(&mut pos, 2)?;
    literal(&mut pos, b':')?;
    let minute = fixed(&mut pos, 2)?;
    literal(&mut pos, b':')?;
    let second = fixed(&mut pos, 2)?;

    // XXX: appendOffset("+HH:MM", "Z").
    let offset_seconds: i64 = match b.get(pos) {
        Some(&s @ (b'+' | b'-')) => {
            let offset_start = pos;
            pos += 1;
            let hours = fixed(&mut pos, 2).map_err(|_| parse_error(text, offset_start))?;
            literal(&mut pos, b':').map_err(|_| parse_error(text, offset_start))?;
            let minutes = fixed(&mut pos, 2).map_err(|_| parse_error(text, offset_start))?;
            if hours > 23 || minutes > 59 {
                return Err(parse_error(text, offset_start));
            }
            let total = hours * 3600 + minutes * 60;
            if s == b'-' { -total } else { total }
        }
        Some(b'Z') => {
            pos += 1;
            0
        }
        _ => return Err(parse_error(text, pos)),
    };
    if pos != b.len() {
        return Err(DateTimeException {
            message: format!(
                "Text '{text}' could not be parsed, unparsed text found at index {pos}"
            ),
        });
    }

    let resolve_error = |message: String| DateTimeException {
        message: format!("Text '{text}' could not be parsed: {message}"),
    };
    // ZoneOffset.ofTotalSeconds
    if offset_seconds.abs() > 18 * 3600 {
        return Err(resolve_error(
            "Zone offset not in valid range: -18:00 to +18:00".to_string(),
        ));
    }
    // IsoChronology.resolveYearOfEra (LENIENT, no era): YEAR = YEAR_OF_ERA.
    // IsoChronology.resolveYMD (LENIENT): YEAR.checkValidIntValue, then roll the month and day.
    if !(-999_999_999..=999_999_999).contains(&year_of_era) {
        return Err(resolve_error(format!(
            "Invalid value for Year (valid values -999999999 - 999999999): {year_of_era}"
        )));
    }
    let year = year_of_era as i64;
    let months_total = year * 12 + (month - 1);
    let first_of_month = days_from_civil(
        months_total.div_euclid(12),
        months_total.rem_euclid(12) + 1,
        1,
    );
    let epoch_day = first_of_month + (day - 1);
    // Parsed.resolveTime (LENIENT): excess days carried into the date.
    let second_of_day = hour * 3600 + minute * 60 + second;
    let epoch_second = epoch_day * SECONDS_PER_DAY + second_of_day - offset_seconds;
    Ok(Instant::of_epoch_second(epoch_second))
}
