//! Bazel's `Printer` rules where they differ from starlark-rust's, for Blaze-owned code paths.
//!
//! Java: net/starlark/java/eval/Printer.java (appendQuoted, debugPrint), Label.debugPrint.
//! `str()`/`repr()` of values are starlark-rust's (`Value::to_str` / `to_repr`); Blaze types
//! implement `collect_repr`/`collect_str` to Bazel's format themselves.

use starlark::values::Value;
use starlark::values::ValueLike;

use crate::label::LabelValue;
use crate::thread::ThreadContext;

/// Java `Printer.appendQuoted`: a Starlark string literal for `s`.
// Java: net/starlark/java/eval/Printer.java:225 Printer.appendQuoted
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Java `Printer.debugPrint(x, thread)`: what `print()` and `fail()` show for an argument.
/// Labels use their display form under the main repo mapping; everything else is `str(x)`.
// Java: net/starlark/java/eval/Printer.java debugPrint; cmdline/Label.java:726 Label.debugPrint
pub fn debug_print(x: Value, ctx: Option<&ThreadContext>) -> String {
    if let Some(l) = x.downcast_ref::<LabelValue>() {
        return l
            .label()
            .debug_print(ctx.and_then(|c| c.main_repo_mapping.as_ref()));
    }
    x.to_str()
}
