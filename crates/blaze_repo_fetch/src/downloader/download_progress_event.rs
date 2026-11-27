//! Port of `DownloadProgressEvent` and the `FetchEvent` build event.

use blaze_events::handler::{FetchProgress, Postable};

use crate::uri::JavaUri;

/// Postable event reporting on progress made downloading an URL.
// Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:27 DownloadProgressEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadProgressEvent {
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:58 DownloadProgressEvent.getOriginalUrl
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:62 DownloadProgressEvent.getResourceIdentifier
    pub original_url: JavaUri,
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:67 DownloadProgressEvent.getActualUrl
    pub actual_url: Option<JavaUri>,
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:76 DownloadProgressEvent.getBytesRead
    pub bytes_read: i64,
    pub total_bytes: Option<i64>,
    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:71 DownloadProgressEvent.isFinished
    pub download_finished: bool,
}

impl Postable for DownloadProgressEvent {
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        Some(self)
    }
}

impl DownloadProgressEvent {
    pub fn new(
        original_url: JavaUri,
        actual_url: Option<JavaUri>,
        bytes_read: i64,
        total_bytes: Option<i64>,
        finished: bool,
    ) -> Self {
        DownloadProgressEvent {
            original_url,
            actual_url,
            bytes_read,
            total_bytes,
            download_finished: finished,
        }
    }
}

impl FetchProgress for DownloadProgressEvent {
    fn get_resource_identifier(&self) -> String {
        self.original_url.to_string()
    }

    // Java: com/google/devtools/build/lib/bazel/repository/downloader/DownloadProgressEvent.java:84 DownloadProgressEvent.getProgress
    fn get_progress(&self) -> String {
        if self.bytes_read > 0 {
            match self.total_bytes {
                Some(total) => {
                    let ratio = if total != 0 {
                        self.bytes_read as f64 / total as f64
                    } else {
                        1.0
                    };
                    // 10.1 MiB (20.2%)
                    format!(
                        "{} ({})",
                        bytes_count_to_display_string(self.bytes_read),
                        format_percentage(ratio)
                    )
                }
                // 10.1 MiB (10,590,000B)
                None => format!(
                    "{} ({}B)",
                    bytes_count_to_display_string(self.bytes_read),
                    format_grouped(self.bytes_read)
                ),
            }
        } else {
            String::new()
        }
    }

    fn is_finished(&self) -> bool {
        self.download_finished
    }
}

/// `DecimalFormat("0.0%", Locale.US)`: the exact binary value, rounded half-even.
fn format_percentage(ratio: f64) -> String {
    let s = format!("{:.3}", ratio);
    let (int_part, frac) = s.split_once('.').unwrap();
    let combined = format!("{}{}", int_part, frac);
    let negative = combined.starts_with('-');
    let digits = combined.trim_start_matches('-').trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    // digits are tenths of a percent.
    let padded = format!("{:0>2}", digits);
    let (a, b) = padded.split_at(padded.len() - 1);
    format!("{}{}.{}%", if negative { "-" } else { "" }, a, b)
}

/// Java `String.format("%,d")` in an English locale.
pub fn format_grouped(n: i64) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{}", out) } else { out }
}

/// Port of `StringUtilities.bytesCountToDisplayString`.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:118 StringUtilities.bytesCountToDisplayString
pub fn bytes_count_to_display_string(bytes: i64) -> String {
    assert!(bytes >= 0, "IllegalArgumentException");
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    let mut unit_index = 0;
    let mut value = bytes;
    while unit_index + 1 < UNITS.len() && value >= (1 << 20) {
        value >>= 10;
        unit_index += 1;
    }
    // DecimalFormat("0.0") of value / 1024.0, HALF_EVEN (the division is exact in binary).
    let num = value as i128 * 10;
    let mut q = num / 1024;
    let r = num % 1024;
    if 2 * r > 1024 || (2 * r == 1024 && q % 2 == 1) {
        q += 1;
    }
    format!("{}.{} {}", q / 10, q % 10, UNITS[unit_index])
}

/// The build event posted after each download attempt (`buildeventstream.FetchEvent`).
// Java: com/google/devtools/build/lib/buildeventstream/FetchEvent.java:27 FetchEvent
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchEvent {
    pub url: String,
    /// `FetchId.Downloader` name, e.g. `HTTP`.
    pub downloader: &'static str,
    pub success: bool,
}

impl Postable for FetchEvent {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes_count_to_display_string(1023), "1023 B");
        assert_eq!(bytes_count_to_display_string(1024), "1.0 KiB");
        assert_eq!(bytes_count_to_display_string(10_590_000), "10.1 MiB");
        assert_eq!(format_grouped(10_590_000), "10,590,000");
        assert_eq!(format_percentage(0.202), "20.2%");
        assert_eq!(format_percentage(1.0), "100.0%");
        assert_eq!(format_percentage(0.0), "0.0%");
    }
}
