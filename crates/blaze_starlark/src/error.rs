//! Locations, syntax errors and evaluation errors, rendered as Bazel renders them.
//!
//! Java: net/starlark/java/syntax/Location.java (toString), net/starlark/java/syntax/
//! SyntaxError.java (toString, Exception.getMessage), net/starlark/java/eval/EvalException.java
//! (getMessageWithStack, formatCallStack).

use std::fmt;

use starlark::codemap::FileSpan;
use starlark::syntax::AstModule;
use starlark::syntax::Dialect;
use starlark::syntax::ast::BinOp;
use starlark::syntax::ast::ExprP;
use starlark::syntax::ast::StmtP;

/// A source position: file, 1-based line and column (0 = unknown, as in Java).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    pub file: String,
    pub line: usize,
    pub column: usize,
}

impl Location {
    /// Java `Location.BUILTIN`.
    pub fn builtin() -> Location {
        Location {
            file: "<builtin>".to_owned(),
            line: 0,
            column: 0,
        }
    }

    /// The start of `span`, 1-based.
    pub fn of_span_start(span: &FileSpan) -> Location {
        let p = span.resolve_span().begin;
        Location {
            file: span.filename().to_owned(),
            line: p.line + 1,
            column: p.column + 1,
        }
    }

    /// Where Bazel reports an error or call at `span`: the operator of a binary expression, the
    /// `.` of a dot expression, the `[` of an index or slice, the `(` of a call, otherwise the
    /// start (Java `BinaryOperatorExpression.getOperatorLocation`, `DotExpression.getDotLocation`,
    /// `IndexExpression.getLbracketLocation`, `CallExpression.getLparenLocation`).
    pub fn bazel_position(span: &FileSpan) -> Location {
        let start = Location::of_span_start(span);
        let text = span.source_span();
        // starlark-rust's span of an expression excludes the parentheses around an operand
        // (`[1] + (1,)` ends before the `)`, `(1, 2)[2]` starts after the `(`): extend the text
        // over them so that it parses alone.
        let (text, start) = match operator_offset(text) {
            Some(_) => (text, start),
            None => balanced_text(span).unwrap_or((text, start)),
        };
        match operator_offset(text) {
            Some(off) => {
                let before = &text[..off];
                let newlines = before.matches('\n').count();
                let column = match before.rfind('\n') {
                    Some(i) => before[i + 1..].chars().count() + 1,
                    None => start.column + before.chars().count(),
                };
                Location {
                    file: start.file,
                    line: start.line + newlines,
                    column,
                }
            }
            None => start,
        }
    }
}

impl fmt::Display for Location {
    // Java: net/starlark/java/syntax/Location.java:76 Location.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file)?;
        if self.line != 0 {
            write!(f, ":{}", self.line)?;
            if self.column != 0 {
                write!(f, ":{}", self.column)?;
            }
        }
        Ok(())
    }
}

/// The source of `span` extended over the unbalanced parentheses at its ends (`(` before it,
/// `)` after it, whitespace and line continuations between), with the location of the new
/// start; `None` when it is balanced.
fn balanced_text(span: &FileSpan) -> Option<(&str, Location)> {
    let src = span.file.source();
    let text = span.source_span();
    let opens = text.matches('(').count() as isize;
    let closes = text.matches(')').count() as isize;
    let mut begin = span.span.begin().get() as usize;
    let mut end = span.span.end().get() as usize;
    let blank = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\\');
    let mut need = opens - closes;
    while need > 0 {
        let mut i = end;
        while i < src.len() && blank(src.as_bytes()[i]) {
            i += 1;
        }
        if src.as_bytes().get(i) != Some(&b')') {
            return None;
        }
        end = i + 1;
        need -= 1;
    }
    while need < 0 {
        let mut i = begin;
        while i > 0 && blank(src.as_bytes()[i - 1]) {
            i -= 1;
        }
        if i == 0 || src.as_bytes()[i - 1] != b'(' {
            return None;
        }
        begin = i - 1;
        need += 1;
    }
    if begin == span.span.begin().get() as usize && end == span.span.end().get() as usize {
        return None;
    }
    let line = span
        .file
        .find_line(starlark::codemap::Pos::new(begin as u32));
    let start = Location {
        file: span.filename().to_owned(),
        line: line + 1,
        column: src[src[..begin].rfind('\n').map_or(0, |i| i + 1)..begin]
            .chars()
            .count()
            + 1,
    };
    Some((&src[begin..end], start))
}

/// Byte offset, within `text` (the source of one expression), of the token Bazel uses as the
/// expression's error location; `None` means the start. The expression is re-parsed alone.
fn operator_offset(text: &str) -> Option<usize> {
    // An augmented or plain assignment statement (Java reports its operator).
    if let Ok(ast) = AstModule::parse("stmt", text.to_owned(), &Dialect::AllOptionsInternal) {
        let lhs_end = match &ast.statement().node {
            StmtP::AssignModify(lhs, _, _) => Some(lhs.span.end().get() as usize),
            StmtP::Assign(a) => Some(a.lhs.span.end().get() as usize),
            _ => None,
        };
        if let Some(from) = lhs_end {
            return text[from..]
                .char_indices()
                .find(|(_, c)| !c.is_ascii_whitespace() && *c != '\\')
                .map(|(i, _)| from + i);
        }
    }
    // Parenthesize so that any expression (including multi-line ones) parses; offsets shift by 1.
    let src = format!("({text}\n)");
    let ast = AstModule::parse("expr", src.clone(), &Dialect::AllOptionsInternal).ok()?;
    let StmtP::Expression(e) = &ast.statement().node else {
        return None;
    };
    let skip_to = |from: usize, want: char| -> Option<usize> {
        let rest = &src[from..];
        let mut in_comment = false;
        for (i, c) in rest.char_indices() {
            if in_comment {
                if c == '\n' {
                    in_comment = false;
                }
                continue;
            }
            if c == '#' {
                in_comment = true;
            } else if c == want {
                return Some(from + i);
            // The `)` closing a parenthesized operand (its span excludes the parentheses).
            } else if !c.is_ascii_whitespace() && c != '\\' && c != ')' {
                return None;
            }
        }
        None
    };
    let pos = match &e.node {
        ExprP::Op(lhs, op, _) => {
            // First non-blank char after the left operand.
            let from = lhs.span.end().get() as usize;
            let first = |from: usize| {
                src[from..]
                    .char_indices()
                    .find(|(_, c)| !c.is_ascii_whitespace() && *c != '\\' && *c != ')')
                    .map(|(i, _)| from + i)
            };
            match (first(from), op) {
                // Java's `not in` operator is located at its `in` token (the parser records the
                // position of the token that completes the operator).
                (Some(at), BinOp::NotIn) if src[at..].starts_with("not") => first(at + 3),
                (at, _) => at,
            }
        }
        ExprP::Dot(obj, _) => skip_to(obj.span.end().get() as usize, '.'),
        ExprP::Index(b) => skip_to(b.0.span.end().get() as usize, '['),
        ExprP::Index2(b) => skip_to(b.0.span.end().get() as usize, '['),
        ExprP::Slice(obj, ..) => skip_to(obj.span.end().get() as usize, '['),
        ExprP::Call(f, _) => skip_to(f.span.end().get() as usize, '('),
        _ => None,
    }?;
    // Undo the leading "(".
    pos.checked_sub(1)
}

/// `f` if `text` is exactly a call `f(...)` of an identifier.
fn call_identifier(text: &str) -> Option<String> {
    let src = format!("({text}\n)");
    let ast = AstModule::parse("expr", src, &Dialect::AllOptionsInternal).ok()?;
    let StmtP::Expression(e) = &ast.statement().node else {
        return None;
    };
    match &e.node {
        ExprP::Call(f, _) => match &f.node {
            ExprP::Identifier(id) => Some(id.node.ident.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A static error: Java `SyntaxError`, printed as `file:line:col: message`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub location: Location,
    pub message: String,
}

impl fmt::Display for SyntaxError {
    // Java: net/starlark/java/syntax/SyntaxError.java:48 SyntaxError.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

/// Java `SyntaxError.Exception.getMessage`: the first message, plus "(+ N more)".
pub fn syntax_errors_message(errors: &[SyntaxError]) -> String {
    match errors {
        [] => String::new(),
        [one] => one.message.clone(),
        [first, rest @ ..] => format!("{} (+ {} more)", first.message, rest.len()),
    }
}

/// One entry of a Bazel call stack (Java `StarlarkThread.CallStackEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEntry {
    /// Function name; `<toplevel>` for the file's top level.
    pub name: String,
    /// Current position in that function; `None` for a builtin (Java `Location.BUILTIN`).
    pub location: Option<Location>,
    /// The source line at `location`, trimmed (Java reads it with a `SourceReader`).
    pub source_line: Option<String>,
}

/// What kind of failure an [`EvalError`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalErrorKind {
    /// Parse, resolve or Bazel syntax-check errors.
    Syntax(Vec<SyntaxError>),
    /// An ordinary evaluation error (Java `EvalException`), including `fail()`.
    Eval,
    /// The Starlark call stack limit was hit.
    StackOverflow,
    /// The cancellation hook fired (Java `InterruptedException`).
    Cancelled,
    /// starlark-rust panicked; contained by `crate::eval`.
    Panic,
}

/// A Starlark failure, with Bazel's structured call stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
    pub kind: EvalErrorKind,
    /// Java `EvalException.getMessage()`.
    pub message: String,
    /// Outermost first, as Java `EvalException.getCallStack()`. Empty for syntax errors.
    pub callstack: Vec<StackEntry>,
    /// False for `fail(..., stack_trace = False)`: render the message alone.
    pub include_stack: bool,
}

impl std::error::Error for EvalError {}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message_with_stack())
    }
}

/// An evaluation error whose call stack is already set (Java: an `EvalException` whose stack was
/// set by another thread, or by `withCallStack`). Java's `EvalException.ensureStack` keeps a
/// stack that is already there, so [`EvalError::from_starlark`] returns this error unchanged when
/// it crosses outer frames (e.g. a symbolic macro's implementation runs in its own thread and
/// its errors carry that thread's stack, MacroClass.executeMacroImplementation).
// Java: net/starlark/java/eval/EvalException.java:127 EvalException.ensureStack
#[derive(Debug, Clone)]
pub struct EvalErrorWithStack(pub EvalError);

impl fmt::Display for EvalErrorWithStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for EvalErrorWithStack {}

impl EvalErrorWithStack {
    /// The error as a starlark-rust error that keeps this stack.
    pub fn into_starlark(self) -> starlark::Error {
        starlark::Error::new_other(anyhow::Error::new(self))
    }

    /// The preset error carried by `e`, if any.
    pub fn of(e: &starlark::Error) -> Option<&EvalError> {
        match e.kind() {
            starlark::ErrorKind::Other(a) | starlark::ErrorKind::Native(a) => {
                a.downcast_ref::<EvalErrorWithStack>().map(|p| &p.0)
            }
            _ => None,
        }
    }
}

/// Message of a starlark-rust error without location or stack (Java `getMessage`).
pub fn bare_message(e: &starlark::Error) -> String {
    if let Some(preset) = EvalErrorWithStack::of(e) {
        return preset.message.clone();
    }
    match e.kind() {
        // Java: net/starlark/java/eval/MethodLibrary.java:888 MethodLibrary.fail: the message is
        // the printed arguments as they are. Blaze's `fail` marks its message; stock `fail`
        // prefixes one space per argument (" msg"), the first of which is not part of it.
        starlark::ErrorKind::Fail(m)
            if m.downcast_ref::<crate::env::FailMessage>().is_some()
                || m.downcast_ref::<crate::env::FailWithoutStack>().is_some() =>
        {
            m.to_string()
        }
        starlark::ErrorKind::Fail(m) => {
            let m = m.to_string();
            m.strip_prefix(' ').map(str::to_owned).unwrap_or(m)
        }
        _ => format!("{:#}", e.without_diagnostic()),
    }
}

/// Java `EvalException.SourceReader`: returns a single line of source code (sans newline), or
/// `None` if unavailable.
// Java: net/starlark/java/eval/EvalException.java:162 EvalException.SourceReader
pub type SourceReader = dyn Fn(&Location) -> Option<String> + Send + Sync;

static SOURCE_READER: std::sync::RwLock<Option<std::sync::Arc<SourceReader>>> =
    std::sync::RwLock::new(None);

/// Sets the function used to obtain the source line of a stack frame when an error's call
/// stack is formatted (Bazel: `BlazeRuntime.newRuntime` installs a reader of the server's file
/// system, so a frame whose file is not on disk, such as the `/virtual_builtins_bzl/...` files
/// of `@_builtins`, prints no source line).
///
/// PORT: Java takes a `Supplier<SourceReader>`; the supplier only makes a fresh reader per
/// formatting, so the reader itself is stored.
// Java: net/starlark/java/eval/EvalException.java:173 EvalException.setSourceReaderSupplier
pub fn set_source_reader_supplier(f: std::sync::Arc<SourceReader>) {
    *SOURCE_READER.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// The source line of a stack frame at `loc`, trimmed.
///
/// PORT: Java reads the line when it formats the stack (`formatCallStack`), Blaze when it
/// converts the error; the stack is formatted from the same files in the same command. Without
/// an installed reader (unit tests, tools), the line is read from the source the error came
/// from; Java's default reader reads `loc.file()` from the file system as UTF-8.
// Java: net/starlark/java/eval/EvalException.java:228 EvalException.formatCallStack
fn source_line(span: &FileSpan, loc: &Location) -> Option<String> {
    let reader = SOURCE_READER
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let line = match reader {
        Some(reader) => reader(loc)?,
        None => {
            if loc.line == 0 {
                return None;
            }
            span.file.source_line(loc.line - 1).to_owned()
        }
    };
    // Java: EvalException.java:229 `line.trim()` (String.trim: chars <= U+0020 only; a line of
    // an internal string may end in U+0085 or U+00A0, which are UTF-8 continuation bytes).
    Some(line.trim_matches(|c: char| c <= ' ').to_owned())
}

impl EvalError {
    pub fn syntax(errors: Vec<SyntaxError>) -> EvalError {
        EvalError {
            message: syntax_errors_message(&errors),
            kind: EvalErrorKind::Syntax(errors),
            callstack: Vec::new(),
            include_stack: true,
        }
    }

    /// Converts a starlark-rust error.
    ///
    /// starlark-rust's `CallStack` is `[Frame{name: callee, location: call site}]` and
    /// `Error::span()` is the failing expression. Java's stack is `[(function, current position
    /// in it)]` starting at `<toplevel>`: entry 0 = (`<toplevel>`, frames[0].location), entry i =
    /// (frames[i-1].name, frames[i].location). The innermost callee is a builtin iff the error
    /// carries no span of its own beyond that call site.
    pub fn from_starlark(e: &starlark::Error) -> EvalError {
        if let Some(preset) = EvalErrorWithStack::of(e) {
            return preset.clone();
        }
        let kind = match e.kind() {
            starlark::ErrorKind::StackOverflow(_) => EvalErrorKind::StackOverflow,
            starlark::ErrorKind::Parser(_) => EvalErrorKind::Syntax(Vec::new()),
            _ if format!("{}", e.without_diagnostic()) == "Evaluation cancelled" => {
                EvalErrorKind::Cancelled
            }
            _ => EvalErrorKind::Eval,
        };
        let message = bare_message(e);
        if let EvalErrorKind::Syntax(_) = kind {
            let location = e
                .span()
                .map(Location::of_span_start)
                .unwrap_or_else(Location::builtin);
            return EvalError::syntax(vec![SyntaxError { location, message }]);
        }
        let entry = |name: &str, span: Option<&FileSpan>| match span {
            Some(s) => {
                let loc = Location::bazel_position(s);
                let line = source_line(s, &loc);
                StackEntry {
                    name: name.to_owned(),
                    location: Some(loc),
                    source_line: line,
                }
            }
            None => StackEntry {
                name: name.to_owned(),
                location: None,
                source_line: None,
            },
        };
        let frames = &e.call_stack().frames;
        let mut callstack = Vec::new();
        if frames.is_empty() {
            callstack.push(entry("<toplevel>", e.span()));
        } else {
            let mut caller = "<toplevel>";
            for (i, f) in frames.iter().enumerate() {
                // A function called from native code (a rule implementation, a transition, a
                // `map_each` callback) is the outermost frame and has no call site: Java's stack
                // starts at that callee (StarlarkThread has no `<toplevel>` frame then), so there
                // is no `File "<builtin>", in <toplevel>` entry.
                if i > 0 || f.location.is_some() {
                    callstack.push(entry(caller, f.location.as_ref()));
                }
                caller = &f.name;
            }
            let last = frames.last().unwrap();
            let builtin = kind != EvalErrorKind::StackOverflow
                && match (e.span(), &last.location) {
                    (None, _) => true,
                    (Some(s), Some(l)) => s == l,
                    (Some(_), None) => false,
                };
            if builtin {
                callstack.push(entry(&last.name, None));
            } else {
                callstack.push(entry(&last.name, e.span()));
            }
        }
        // starlark-rust compiles some builtins (`len`, `type`, …) to intrinsics that push no
        // frame. An error whose span is a whole call `f(...)` was raised by that call itself (an
        // argument's error would carry the argument's span), so it is a builtin leaf.
        if kind == EvalErrorKind::Eval
            && callstack.last().is_some_and(|l| l.location.is_some())
            && let Some(name) = e.span().and_then(|s| call_identifier(s.source_span()))
        {
            callstack.push(entry(&name, None));
        }
        let include_stack = !matches!(e.kind(), starlark::ErrorKind::Fail(a)
            if a.downcast_ref::<crate::env::FailWithoutStack>().is_some());
        EvalError {
            kind,
            message,
            callstack,
            include_stack,
        }
    }

    /// The innermost frame's location, if it is not a builtin (Java `EvalException` location
    /// used by event reporting).
    pub fn location(&self) -> Option<&Location> {
        match &self.kind {
            EvalErrorKind::Syntax(errs) => errs.first().map(|e| &e.location),
            _ => self
                .callstack
                .iter()
                .rev()
                .find_map(|e| e.location.as_ref()),
        }
    }

    /// Reads the source line of each frame that has a location but no source line, as Java's
    /// default `EvalException.SourceReader` does when it formats the stack (the file read as
    /// UTF-8, its `line`th line; nothing if the file cannot be read). Used for stacks Blaze
    /// assembles from recorded call stacks (e.g. a symbolic macro's instantiation stack).
    // PORT: Java reads the lines when formatting; they are read when the stack is assembled.
    // Java: net/starlark/java/eval/EvalException.java:182 EvalException.sourceReaderSupplier
    pub fn read_missing_source_lines(&mut self) {
        for entry in &mut self.callstack {
            if entry.source_line.is_some() {
                continue;
            }
            let Some(loc) = &entry.location else {
                continue;
            };
            if loc.line == 0 {
                continue;
            }
            // Java formats with the installed reader (EvalException.setSourceReaderSupplier).
            let reader = SOURCE_READER
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(reader) = reader {
                entry.source_line =
                    reader(loc).map(|l| l.trim_matches(|c: char| c <= ' ').to_owned());
                continue;
            }
            // The file name is an internal string (one char per byte): its bytes are the
            // platform path (blaze_util::string_encoding::internal_to_platform).
            let path: Vec<u8> = loc
                .file
                .chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                .collect();
            let path = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(&path);
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            // Java: Files.asCharSource(file, UTF_8).read(), split on "\n", then trim().
            let content = String::from_utf8_lossy(&bytes);
            entry.source_line = content
                .split('\n')
                .nth(loc.line - 1)
                .map(|l| l.trim_matches(|c: char| c <= ' ').to_owned());
        }
    }

    /// Java `EvalException.getMessageWithStack` / `formatCallStack`. Syntax errors render one
    /// `file:line:col: message` per line.
    // Java: net/starlark/java/eval/EvalException.java:201 EvalException.formatCallStack
    pub fn message_with_stack(&self) -> String {
        if let EvalErrorKind::Syntax(errs) = &self.kind {
            return errs
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n");
        }
        if !self.include_stack {
            return self.message.clone();
        }
        // Java formatCallStack reads each frame's source line with the SourceReader when it
        // formats (EvalException.java:225); a frame of a stack assembled without them (a builtin's
        // `StarlarkThread.getCallStack`, a recorded call stack) gets it here.
        if self
            .callstack
            .iter()
            .any(|e| e.source_line.is_none() && e.location.as_ref().is_some_and(|l| l.line != 0))
        {
            let mut filled = self.clone();
            filled.read_missing_source_lines();
            if filled.callstack != self.callstack {
                return filled.message_with_stack();
            }
        }
        let mut n = self.callstack.len();
        if n == 0 {
            return format!("Error: {}", self.message);
        }
        let mut prefix = "Error: ".to_owned();
        let leaf = &self.callstack[n - 1];
        if leaf.location.is_none() {
            prefix = format!("Error in {}: ", leaf.name);
            n -= 1;
        }
        let mut buf = String::new();
        if n > 0 {
            buf.push_str("Traceback (most recent call last):\n");
            for fr in &self.callstack[..n] {
                match &fr.location {
                    Some(loc) => {
                        buf.push_str(&format!("\tFile \"{}\", ", loc.file));
                        if loc.line != 0 {
                            buf.push_str(&format!("line {}, ", loc.line));
                            if loc.column != 0 {
                                buf.push_str(&format!("column {}, ", loc.column));
                            }
                        }
                    }
                    None => buf.push_str("\tFile \"<builtin>\", "),
                }
                buf.push_str(&format!("in {}\n", fr.name));
                if let Some(line) = &fr.source_line {
                    buf.push_str(&format!("\t\t{line}\n"));
                }
            }
        }
        buf.push_str(&prefix);
        buf.push_str(&self.message);
        buf
    }
}
