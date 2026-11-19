//! Helpers shared by the .bzl rule-definition builtins: Java `Starlark.errorf`,
//! `Sequence.cast`, `Dict.cast`, `Starlark.trimDocString`, `Identifier.isValid` and
//! `StarlarkThread.getCallStack`.
//!
//! PORT: move to blaze_starlark (these are net.starlark.java helpers); kept here until the
//! owner of blaze_starlark takes them.

use std::fmt;

use blaze_cmdline::EvalException;
use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;
use starlark::eval::Evaluator;
use starlark::values::Value;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;

/// Java `Starlark.errorf(...)` / `new EvalException(msg)` raised by a builtin.
pub fn errorf(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// Converts a `blaze_cmdline::EvalException` (thrown by model code) to a Starlark error.
pub fn eval_exception(e: EvalException) -> starlark::Error {
    errorf(e.get_message())
}

/// Java `Starlark.type(x)`.
pub fn starlark_type(v: Value) -> &'static str {
    v.get_type()
}

/// Java `Starlark.isNullOrNone`.
pub fn is_none(v: Value) -> bool {
    v.is_none()
}

/// The elements of a Starlark sequence (list or tuple), or `None` if `v` is not one (Java
/// `instanceof Sequence`).
pub fn sequence_elements<'v>(v: Value<'v>) -> Option<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Some(t.content().to_vec());
    }
    None
}

/// Java `Sequence.cast(x, elemType, what)`: `want` is `Starlark.classType(elemType)` and
/// `accepts` the `isAssignableFrom` test.
// Java: net/starlark/java/eval/Sequence.java:132 Sequence.cast
pub fn sequence_cast<'v>(
    x: Value<'v>,
    want: &str,
    accepts: impl Fn(Value<'v>) -> bool,
    what: &str,
) -> starlark::Result<Vec<Value<'v>>> {
    let Some(elems) = sequence_elements(x) else {
        return Err(errorf(format!(
            "for {what}, got {}, want sequence",
            starlark_type(x)
        )));
    };
    for (i, elem) in elems.iter().enumerate() {
        if !accepts(*elem) {
            return Err(errorf(format!(
                "at index {i} of {what}, got element of type {}, want {want}",
                starlark_type(*elem)
            )));
        }
    }
    Ok(elems)
}

/// Java `Sequence.noneableCast`: `None` is the empty list.
// Java: net/starlark/java/eval/Sequence.java:153 Sequence.noneableCast
pub fn sequence_noneable_cast<'v>(
    x: Value<'v>,
    want: &str,
    accepts: impl Fn(Value<'v>) -> bool,
    what: &str,
) -> starlark::Result<Vec<Value<'v>>> {
    if x.is_none() {
        return Ok(Vec::new());
    }
    sequence_cast(x, want, accepts, what)
}

/// Java `Sequence.cast(x, String.class, what)`.
pub fn sequence_cast_strings(x: Value, what: &str) -> starlark::Result<Vec<String>> {
    Ok(
        sequence_cast(x, "string", |e| e.unpack_str().is_some(), what)?
            .into_iter()
            .map(|e| e.unpack_str().unwrap().to_owned())
            .collect(),
    )
}

/// Java `Sequence.noneableCast(x, String.class, what)`.
pub fn sequence_noneable_cast_strings(x: Value, what: &str) -> starlark::Result<Vec<String>> {
    if x.is_none() {
        return Ok(Vec::new());
    }
    sequence_cast_strings(x, what)
}

/// The entries of a Starlark dict in insertion order, or `None` if `v` is not a dict.
pub fn dict_entries<'v>(v: Value<'v>) -> Option<Vec<(Value<'v>, Value<'v>)>> {
    DictRef::from_value(v).map(|d| d.iter().collect())
}

/// Java `Dict.cast(x, keyType, valueType, what)`; `key_want`/`value_want` are the
/// `Starlark.classType` names.
// Java: net/starlark/java/eval/Dict.java:659 Dict.cast
pub fn dict_cast<'v>(
    x: Value<'v>,
    key_want: &str,
    key_accepts: impl Fn(Value<'v>) -> bool,
    value_want: &str,
    value_accepts: impl Fn(Value<'v>) -> bool,
    what: &str,
) -> starlark::Result<Vec<(Value<'v>, Value<'v>)>> {
    let Some(entries) = dict_entries(x) else {
        return Err(errorf(format!(
            "got {} for '{what}', want dict",
            starlark_type(x)
        )));
    };
    for (k, v) in &entries {
        if !key_accepts(*k) || !value_accepts(*v) {
            return Err(errorf(format!(
                "got dict<{}, {}> for '{what}', want dict<{key_want}, {value_want}>",
                starlark_type(*k),
                starlark_type(*v)
            )));
        }
    }
    Ok(entries)
}

/// Java `Dict.cast(x, String.class, String.class, what)`.
pub fn dict_cast_string_string(x: Value, what: &str) -> starlark::Result<Vec<(String, String)>> {
    Ok(dict_cast(
        x,
        "string",
        |k| k.unpack_str().is_some(),
        "string",
        |v| v.unpack_str().is_some(),
        what,
    )?
    .into_iter()
    .map(|(k, v)| {
        (
            k.unpack_str().unwrap().to_owned(),
            v.unpack_str().unwrap().to_owned(),
        )
    })
    .collect())
}

/// Java `Identifier.isValid`.
// Java: net/starlark/java/syntax/Identifier.java:82 Identifier.isValid
pub fn is_valid_identifier(name: &str) -> bool {
    for (i, c) in name.chars().enumerate() {
        if !(c.is_ascii_lowercase()
            || c.is_ascii_uppercase()
            || (i > 0 && c.is_ascii_digit())
            || c == '_')
        {
            return false;
        }
    }
    !name.is_empty()
}

// Java: net/starlark/java/eval/Starlark.java:640 Starlark.expandTabs
fn expand_tabs(line: &str, tab_size: usize) -> String {
    if !line.contains('\t') {
        return line.to_owned();
    }
    let mut result = String::new();
    let mut col = 0usize;
    for c in line.chars() {
        match c {
            '\n' | '\r' => {
                col = 0;
                result.push(c);
            }
            '\t' => {
                let spaces = tab_size - col % tab_size;
                for _ in 0..spaces {
                    result.push(' ');
                }
                col += spaces;
            }
            _ => {
                result.push(c);
                col += 1;
            }
        }
    }
    result
}

/// Java `String.lines()`: split on `\n`, `\r` or `\r\n`, no trailing empty line.
fn java_lines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(&s[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push(&s[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

/// `StringModule.LATIN1_WHITESPACE`, the default `chars` of `strip` under
/// `StarlarkSemantics.DEFAULT` (`trimDocString` passes DEFAULT, where
/// `-internal_bazel_only_utf_8_byte_strings` is false). Unlike `char::is_whitespace` it holds
/// U+001C..U+001F.
// Java: net/starlark/java/eval/StringModule.java:196 StringModule.LATIN1_WHITESPACE
fn is_starlark_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\n'
            | '\u{000B}'
            | '\u{000C}'
            | '\r'
            | '\u{001C}'
            | '\u{001D}'
            | '\u{001E}'
            | '\u{001F}'
            | ' '
            | '\u{0085}'
            | '\u{00A0}'
    )
}

/// Java `Starlark.trimDocString`.
// Java: net/starlark/java/eval/Starlark.java:586 Starlark.trimDocString
pub fn trim_doc_string(doc_string: &str) -> String {
    let expanded = expand_tabs(doc_string, 8);
    let lines = java_lines(&expanded);
    if lines.is_empty() {
        return String::new();
    }
    let first_line_trimmed = lines[0].trim_matches(is_starlark_whitespace);
    let subsequent = &lines[1..];
    let mut min_leading_spaces = usize::MAX;
    for line in subsequent {
        let stripped_leading = line.trim_start_matches(is_starlark_whitespace);
        if !stripped_leading.is_empty() {
            let leading_spaces = line.chars().count() - stripped_leading.chars().count();
            min_leading_spaces = min_leading_spaces.min(leading_spaces);
        }
    }
    if min_leading_spaces == usize::MAX {
        min_leading_spaces = 0;
    }
    let mut result = String::new();
    result.push_str(first_line_trimmed);
    for line in subsequent {
        // Length check ensures we ignore leading empty lines
        if !result.is_empty() {
            result.push('\n');
        }
        if line.chars().count() > min_leading_spaces {
            let rest: String = line.chars().skip(min_leading_spaces).collect();
            result.push_str(rest.trim_end_matches(is_starlark_whitespace));
        }
    }
    // Remove trailing empty lines
    result.trim_end_matches(is_starlark_whitespace).to_owned()
}

/// Java `Starlark.toJavaOptional(doc, String.class).map(Starlark::trimDocString)`.
pub fn optional_doc(doc: Option<&str>) -> Option<String> {
    doc.map(trim_doc_string)
}

/// Java `StarlarkThread.getCallerLocation()` inside a builtin: the location of the call
/// (its `(`), or `Location.BUILTIN`.
// Java: net/starlark/java/eval/StarlarkThread.java StarlarkThread.getCallerLocation
pub fn caller_location(eval: &Evaluator) -> Location {
    blaze_starlark::env::caller_location(eval).unwrap_or_else(Location::builtin)
}

fn stack_entry(name: &str, span: Option<&starlark::codemap::FileSpan>) -> StackEntry {
    StackEntry {
        name: name.to_owned(),
        location: span.map(Location::bazel_position),
        // PORT: Java's CallStackEntry carries no source line; tracebacks read it separately.
        source_line: None,
    }
}

/// Java `StarlarkThread.getCallStack()` from inside a builtin: `<toplevel>` at the outermost
/// call, each function at its current call, then the called builtin (location BUILTIN) last.
// Java: net/starlark/java/eval/StarlarkThread.java:603 StarlarkThread.getCallStack
pub fn call_stack(eval: &Evaluator) -> Vec<StackEntry> {
    let frames = eval.call_stack().frames;
    let mut stack = Vec::with_capacity(frames.len() + 1);
    let mut caller = "<toplevel>".to_owned();
    for frame in &frames {
        stack.push(stack_entry(&caller, frame.location.as_ref()));
        caller = frame.name.clone();
    }
    if !frames.is_empty() {
        stack.push(stack_entry(&caller, None));
    }
    stack
}

/// Java's Starlark call-machinery error for an argument of a wrong type.
// Java: net/starlark/java/eval/BuiltinFunction.java BuiltinFunction.getArgumentVector
pub fn param_type_error(func: &str, param: &str, v: Value, want: &str) -> starlark::Error {
    errorf(format!(
        "in call to {func}(), parameter '{param}' got value of type '{}', want '{want}'",
        starlark_type(v)
    ))
}

/// Java `BazelModuleContext.ofInnermostBzlOrThrow(thread)`.
// Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:160 BazelModuleContext.ofInnermostBzlOrThrow
pub fn module_context_or_throw(
    eval: &Evaluator,
) -> starlark::Result<std::sync::Arc<crate::bzl_context::BazelModuleContext>> {
    crate::bzl_context::BazelModuleContext::of_innermost_bzl(eval).ok_or_else(|| {
        errorf(
            "internal error: no .bzl module on the call stack \
             (BazelModuleContext.ofInnermostBzlOrThrow)",
        )
    })
}

/// Java `LabelConverter.forBzlEvaluatingThread(thread)`.
// Java: com/google/devtools/build/lib/packages/LabelConverter.java:41 LabelConverter.forBzlEvaluatingThread
pub fn label_converter_for_bzl(eval: &Evaluator) -> starlark::Result<crate::types::LabelConverter> {
    // Fail with an error (not a panic) outside .bzl code.
    module_context_or_throw(eval)?;
    crate::types::LabelConverter::for_bzl_evaluating_thread(eval).map_err(eval_exception)
}

/// Java `BuiltinRestriction.failIfCalledOutsideBuiltins`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:102 BuiltinRestriction.failIfCalledOutsideBuiltins
pub fn fail_if_called_outside_builtins(eval: &Evaluator) -> starlark::Result<()> {
    let current_file = module_context_or_throw(eval)?.label().clone();
    if current_file.get_repository().get_name() != "_builtins" {
        return Err(errorf(format!(
            "file '{}' cannot use private @_builtins API",
            current_file.get_canonical_form()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_doc_string_strips_indentation() {
        assert_eq!(
            trim_doc_string("Heading\n\n    Details paragraph\n    "),
            "Heading\n\nDetails paragraph"
        );
        assert_eq!(
            trim_doc_string("\n    Heading\n\n    Details paragraph\n    "),
            "Heading\n\nDetails paragraph"
        );
        assert_eq!(trim_doc_string(""), "");
    }

    // Not a Java test: Starlark.trimDocString strips StringModule.LATIN1_WHITESPACE, which holds
    // U+001C..U+001F, U+0085 and U+00A0 (bytes of internal strings) but not U+2000.
    #[test]
    fn trim_doc_string_strips_latin1_whitespace() {
        assert_eq!(
            trim_doc_string("\u{1c}Heading\u{1f}\n\n  \u{a0}Details\u{85}"),
            "Heading\n\nDetails"
        );
        // `é` is C3 A9 and `Å` is C3 85: only the trailing 85 is whitespace.
        assert_eq!(
            trim_doc_string("\u{c3}\u{a9} \u{c3}\u{85}"),
            "\u{c3}\u{a9} \u{c3}"
        );
    }

    #[test]
    fn identifiers() {
        assert!(is_valid_identifier("a_1"));
        assert!(is_valid_identifier("_x"));
        assert!(!is_valid_identifier("1a"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("a-b"));
    }
}
