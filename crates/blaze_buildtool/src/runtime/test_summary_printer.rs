//! Port of `runtime/TestSummaryPrinter.java`: prints test statistics in human readable form.

use std::sync::Arc;

use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::repository_mapping::EMPTY;
use blaze_events::{AnsiTerminalPrinter, Mode, OutputStream};
use blaze_optdefs::command::execution_options::TestOutputFormat;
use blaze_protos::blaze::test_case::Status as TestCaseStatus;
use blaze_protos::blaze::{BlazeTestStatus, FailedTestCasesStatus, TestCase};
use blaze_vfs::Path;

use super::test_summary::TestSummary;
use blaze_exec::exec::test_log_helper;

/// The `AnsiTerminalPrinter` methods the test summary classes call.
///
/// PORT: Java's tests mock `AnsiTerminalPrinter` to see the raw strings (with their mode
/// markers); it is a trait here so tests can record them the same way.
// Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:42 AnsiTerminalPrinter
pub trait TerminalPrinter: Send + Sync {
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:95 AnsiTerminalPrinter.print
    fn print(&self, str: &str);
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:114 AnsiTerminalPrinter.printLn
    fn print_ln(&self, str: &str);
    // Java: com/google/devtools/build/lib/util/io/AnsiTerminalPrinter.java:121 AnsiTerminalPrinter.getOutputStream
    fn get_output_stream(&self) -> Arc<dyn OutputStream>;
}

impl TerminalPrinter for AnsiTerminalPrinter {
    fn print(&self, str: &str) {
        AnsiTerminalPrinter::print(self, str)
    }

    fn print_ln(&self, str: &str) {
        AnsiTerminalPrinter::print_ln(self, str)
    }

    fn get_output_stream(&self) -> Arc<dyn OutputStream> {
        AnsiTerminalPrinter::get_output_stream(self).clone()
    }
}

/// Interface for getting the `String` to display to the user for a `Path` corresponding to a
/// test output (e.g. test log).
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:44 TestSummaryPrinter.TestLogPathFormatter
pub trait TestLogPathFormatter: Send + Sync {
    // Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:45 TestLogPathFormatter.getPathStringToPrint
    fn get_path_string_to_print(&self, path: &Path) -> String;
}

impl<F: Fn(&Path) -> String + Send + Sync> TestLogPathFormatter for F {
    fn get_path_string_to_print(&self, path: &Path) -> String {
        self(path)
    }
}

/// Java `Strings.padEnd(string, minLength, padChar)` (lengths in chars).
fn pad_end(string: &str, min_length: i32, pad_char: char) -> String {
    let mut s = string.to_string();
    let mut len = string.chars().count() as i32;
    while len < min_length {
        s.push(pad_char);
        len += 1;
    }
    s
}

/// Java `String.format(Locale.US, "%.<precision>f", d)`: rounded half-up from the shortest
/// decimal representation (Java's `FormattedFloatingDecimal`), as
/// `blaze_optdefs::common::resource_converter::java_format_f` does for six digits.
pub(crate) fn java_format_fixed(d: f64, precision: usize) -> String {
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
    if frac_len > precision {
        let round_up = digits[int_len + precision] >= b'5';
        digits.truncate(int_len + precision);
        frac_len = precision;
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
    if precision > 0 {
        out.push('.');
        out.push_str(std::str::from_utf8(&digits[total_int..]).unwrap());
        for _ in frac_len..precision {
            out.push('0');
        }
    }
    out
}

/// `printer.getOutputStream()` as the `java.io.OutputStream` that `TestLogHelper.writeTestLog`
/// writes to.
struct OutputStreamWriter(Arc<dyn OutputStream>);

impl std::io::Write for OutputStreamWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// `BlazeTestStatus.toString()`: the enum constant's name.
pub fn blaze_test_status_name(status: BlazeTestStatus) -> &'static str {
    status.as_str_name()
}

/// Print the cached test log to the given printer.
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:49 TestSummaryPrinter.printCachedOutput
pub fn print_cached_output(
    summary: &TestSummary,
    test_output: TestOutputFormat,
    printer: &dyn TerminalPrinter,
    test_log_path_formatter: &dyn TestLogPathFormatter,
    max_test_output_bytes: i32,
) {
    let test_name = summary.get_label().to_string();
    let mut all_logs: Vec<String> = Vec::new();
    for path in summary.get_failed_logs() {
        all_logs.push(test_log_path_formatter.get_path_string_to_print(path));
    }
    for path in summary.get_passed_logs() {
        all_logs.push(test_log_path_formatter.get_path_string_to_print(path));
    }
    printer.print_ln(&format!(
        "{}{}: {}{} (see {})",
        summary.get_status_mode(),
        blaze_test_status_name(summary.get_status()),
        Mode::Default,
        test_name,
        all_logs.join(" ")
    ));
    printer.print_ln(&format!(
        "{}INFO: {}From Testing {}",
        Mode::Info,
        Mode::Default,
        test_name
    ));

    // Whether to output the target at all was checked by the caller.
    // Now check whether to output failing shards.
    if test_log_helper::should_output_test_log(test_output, false) {
        for path in summary.get_failed_logs() {
            if test_log_helper::write_test_log(
                path,
                &test_name,
                &mut OutputStreamWriter(printer.get_output_stream()),
                max_test_output_bytes,
            )
            .is_err()
            {
                printer.print_ln(&format!(
                    "==================== Could not read test output for {}",
                    test_name
                ));
            }
        }
    }

    // And passing shards, independently.
    if test_log_helper::should_output_test_log(test_output, true) {
        for path in summary.get_passed_logs() {
            if test_log_helper::write_test_log(
                path,
                &test_name,
                &mut OutputStreamWriter(printer.get_output_stream()),
                max_test_output_bytes,
            )
            .is_err()
            {
                printer.print_ln(&format!(
                    "==================== Could not read test output for {}",
                    test_name
                ));
            }
        }
    }
}

// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:100 TestSummaryPrinter.statusString
fn status_string(summary: &TestSummary) -> String {
    if summary.is_skipped() {
        // If the test was skipped then its status will be something like NO_STATUS. That's not
        // informative enough to a user. Instead, return "SKIPPED" for skipped tests.
        return "SKIPPED".to_string();
    }
    blaze_test_status_name(summary.get_status()).replace('_', " ")
}

/// Prints summary status for a single test.
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:114 TestSummaryPrinter.print
pub fn print(
    summary: &TestSummary,
    terminal_printer: &dyn TerminalPrinter,
    test_log_path_formatter: &dyn TestLogPathFormatter,
    verbose_summary: bool,
    show_all_test_cases: bool,
) {
    print_with_configuration(
        summary,
        terminal_printer,
        test_log_path_formatter,
        verbose_summary,
        show_all_test_cases,
        false,
        &EMPTY,
    );
}

/// Prints summary status for a single test.
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:135 TestSummaryPrinter.print
pub fn print_with_configuration(
    summary: &TestSummary,
    terminal_printer: &dyn TerminalPrinter,
    test_log_path_formatter: &dyn TestLogPathFormatter,
    verbose_summary: bool,
    show_all_test_cases: bool,
    with_configuration_name: bool,
    main_repo_mapping: &RepositoryMapping,
) {
    let status = summary.get_status();
    // Skip output for tests that failed to build.
    if (!verbose_summary && status == BlazeTestStatus::FailedToBuild)
        || status == BlazeTestStatus::BlazeHaltedBeforeTesting
    {
        return;
    }
    let message = format!("{}{}", get_cache_message(summary), status_string(summary));
    let mut target_name = summary
        .get_label()
        .get_display_form(Some(main_repo_mapping));
    if with_configuration_name {
        let mnemonic = match summary.get_configuration() {
            Some(c) => c.get_mnemonic(),
            None => panic!("NullPointerException: configuration"),
        };
        target_name += &format!(" ({})", mnemonic);
    }
    terminal_printer.print(&format!(
        "{} {}{}{}{}\n",
        pad_end(&target_name, 78 - message.chars().count() as i32, ' '),
        summary.get_status_mode(),
        message,
        Mode::Default,
        if verbose_summary {
            get_attempt_summary(summary) + &get_time_summary(summary)
        } else {
            String::new()
        }
    ));

    if show_all_test_cases {
        for test_case in summary.get_passed_test_cases() {
            print_test_case(terminal_printer, test_case);
        }
        for test_case in summary.get_skipped_test_cases() {
            print_test_case(terminal_printer, test_case);
        }

        if summary.get_status() == BlazeTestStatus::Failed {
            if summary.get_failed_test_cases_status() == Some(FailedTestCasesStatus::NotAvailable) {
                terminal_printer.print(&format!(
                    "{}    (individual test case information not available) {}\n",
                    Mode::Warning,
                    Mode::Default
                ));
            } else {
                for test_case in summary.get_failed_test_cases() {
                    if test_case.status() != TestCaseStatus::Passed {
                        print_test_case(terminal_printer, test_case);
                    }
                }

                if summary.get_failed_test_cases_status() != Some(FailedTestCasesStatus::Full) {
                    terminal_printer.print(&format!(
                        "{}    (some shards did not report details, list of failed test cases incomplete)\n{}",
                        Mode::Warning,
                        Mode::Default
                    ));
                }
            }
        }
    } else {
        for warning in summary.get_warnings() {
            terminal_printer.print(&format!(
                "  {}WARNING: {}{}\n",
                Mode::Warning,
                Mode::Default,
                warning
            ));
        }

        for path in summary.get_failed_logs() {
            if path.exists() {
                terminal_printer.print(&format!(
                    "  {}\n",
                    test_log_path_formatter.get_path_string_to_print(path)
                ));
            }
        }
    }
    for path in summary.get_coverage_files() {
        // Print only non-trivial coverage files.
        if path.exists() {
            match path.get_file_size() {
                Ok(size) if size > 0 => {
                    terminal_printer.print(&format!(
                        "  {}\n",
                        test_log_path_formatter.get_path_string_to_print(path)
                    ));
                }
                Ok(_) => {}
                Err(_e) => {
                    // PORT: skipped, logging (`LoggingUtil.logToRemote(Level.WARNING, "Error
                    // while reading coverage data file size", e)`).
                }
            }
        }
    }
}

/// Prints the result of an individual test case.
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:220 TestSummaryPrinter.printTestCase
pub fn print_test_case(terminal_printer: &dyn TerminalPrinter, test_case: &TestCase) {
    let time_summary = match test_case.run_duration_millis {
        Some(ms) => format!(" ({})", time_in_sec_millis(ms)),
        None => String::new(),
    };

    let mode = match test_case.status() {
        TestCaseStatus::Passed => Mode::Info,
        TestCaseStatus::Skipped => Mode::Warning,
        _ => Mode::Error,
    };
    terminal_printer.print(&format!(
        "    {}{}{}{}.{}{}\n",
        mode,
        pad_end(test_case.status().as_str_name(), 8, ' '),
        Mode::Default,
        test_case.class_name(),
        test_case.name(),
        time_summary
    ));
}

/// Return the given time in seconds, to 1 decimal place, i.e. "32.1s".
///
/// PORT: Java takes a `TimeUnit`; every caller passes `MILLISECONDS`.
// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:252 TestSummaryPrinter.timeInSec
pub fn time_in_sec_millis(time: i64) -> String {
    let ms = time as f64;
    format!("{}s", java_format_fixed(ms / 1000.0, 1))
}

// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:257 TestSummaryPrinter.getAttemptSummary
pub fn get_attempt_summary(summary: &TestSummary) -> String {
    let attempts = summary.get_passed_logs().len() + summary.get_failed_logs().len();
    if attempts > 1 {
        // Print number of failed runs for failed tests if testing was completed.
        if summary.get_status() == BlazeTestStatus::Flaky {
            return format!(
                ", failed in {} out of {}",
                summary.get_failed_logs().len(),
                attempts
            );
        }
        if summary.get_status() == BlazeTestStatus::Timeout
            || summary.get_status() == BlazeTestStatus::Failed
        {
            return format!(
                " in {} out of {}",
                summary.get_failed_logs().len(),
                attempts
            );
        }
    }
    String::new()
}

// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:272 TestSummaryPrinter.getCacheMessage
pub fn get_cache_message(summary: &TestSummary) -> String {
    if summary.get_num_cached() == 0
        || summary.get_status() == BlazeTestStatus::Incomplete
        || summary.get_status() == BlazeTestStatus::NoStatus
        || summary.get_status() == BlazeTestStatus::FailedToBuild
    {
        String::new() // either no caching, or information isn't useful
    } else if summary.get_num_cached() == summary.total_runs() {
        "(cached) ".to_string()
    } else {
        format!(
            "({}/{} cached) ",
            summary.get_num_cached(),
            summary.total_runs()
        )
    }
}

// Java: com/google/devtools/build/lib/runtime/TestSummaryPrinter.java:286 TestSummaryPrinter.getTimeSummary
pub fn get_time_summary(summary: &TestSummary) -> String {
    let test_times = summary.get_test_times();
    if test_times.is_empty()
        || summary.get_status() == BlazeTestStatus::NoStatus
        || summary.get_status() == BlazeTestStatus::FailedToBuild
    {
        String::new() // either no tests ran, or information isn't useful
    } else if test_times.len() == 1 {
        format!(" in {}", time_in_sec_millis(test_times[0]))
    } else {
        // We previously used com.google.math for this, which added about 1 MB of deps to the total
        // size. If we re-introduce a dependency on that package, we could revert this change.
        let mut min = test_times[0];
        let mut max = min;
        let mut sum: i64 = 0;
        let mut sum_of_squares = 0.0f64;
        for &value in test_times {
            min = value.min(min);
            max = value.max(max);
            sum = sum.wrapping_add(value);
            sum_of_squares += (value as f64) * (value as f64);
        }
        let mean = (sum as f64) / test_times.len() as f64;
        let stddev = ((sum_of_squares - sum as f64 * mean) / test_times.len() as f64).sqrt();
        // For sharded tests, we print the max time on the same line as
        // the test, and then print more detailed info about the
        // distribution of times on the next line.
        let max_time = time_in_sec_millis(max);
        format!(
            " in {}\n  Stats over {} runs: max = {}, min = {}, avg = {}, dev = {}",
            max_time,
            test_times.len(),
            max_time,
            time_in_sec_millis(min),
            time_in_sec_millis(mean as i64),
            time_in_sec_millis(stddev as i64)
        )
    }
}
