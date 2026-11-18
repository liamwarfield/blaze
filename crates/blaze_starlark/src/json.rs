//! The `json` module.
//!
//! Java: net/starlark/java/lib/json/Json.java (encode, decode, indent, encode_indent), ported
//! with its error texts. Strings are sequences of Unicode scalar values (docs/decisions.md:
//! code-point semantics), so offsets in decode errors count characters.

use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::dict::AllocDict;
use starlark::values::dict::DictRef;
use starlark::values::list::AllocList;

use crate::printer::quote;
use crate::structs::StructureMarker;

/// Nesting limit standing in for Java's StackOverflowError ("nesting depth limit exceeded").
pub const MAX_DEPTH: usize = 1000;

fn err(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

const HEX: &[u8; 16] = b"0123456789abcdef";

// -------------------------------------------------------------------------------------------
// Encoder

struct Encoder<'v> {
    out: String,
    heap: Heap<'v>,
    depth: usize,
}

/// Raised internally when [`MAX_DEPTH`] is exceeded; never wrapped with context.
struct TooDeep;

enum EncErr {
    Msg(String),
    TooDeep,
}

impl From<TooDeep> for EncErr {
    fn from(_: TooDeep) -> Self {
        EncErr::TooDeep
    }
}

impl<'v> Encoder<'v> {
    // Java: Json.java:142 Encoder.encode
    fn encode(&mut self, x: Value<'v>) -> Result<(), EncErr> {
        if self.depth >= MAX_DEPTH {
            return Err(TooDeep.into());
        }
        self.depth += 1;
        let r = self.encode_inner(x);
        self.depth -= 1;
        r
    }

    fn encode_inner(&mut self, x: Value<'v>) -> Result<(), EncErr> {
        if x.is_none() {
            self.out.push_str("null");
            return Ok(());
        }
        if let Some(s) = x.unpack_str() {
            append_quoted(&mut self.out, s);
            return Ok(());
        }
        if let Some(b) = x.unpack_bool() {
            self.out.push_str(if b { "true" } else { "false" });
            return Ok(());
        }
        match x.get_type() {
            "int" => {
                self.out.push_str(&x.to_str());
                return Ok(());
            }
            "float" => {
                let r = x.to_repr();
                let f: f64 = r.parse().unwrap_or(f64::NAN);
                if !f.is_finite() {
                    return Err(EncErr::Msg(format!("cannot encode non-finite float {r}")));
                }
                // Always contains a decimal point or exponent.
                self.out.push_str(&r);
                return Ok(());
            }
            _ => {}
        }

        // e.g. dict (must have string keys)
        if let Some(d) = DictRef::from_value(x) {
            let entries: Vec<(Value<'v>, Value<'v>)> = d.iter().collect();
            drop(d);
            let mut keyed: Vec<(&str, Value<'v>)> = Vec::with_capacity(entries.len());
            for (k, _) in &entries {
                if k.unpack_str().is_none() {
                    return Err(EncErr::Msg(format!(
                        "{} has {} key, want string",
                        x.get_type(),
                        k.get_type()
                    )));
                }
            }
            for (k, v) in &entries {
                keyed.push((k.unpack_str().unwrap(), *v));
            }
            // Sort keys for determinism (Java: Arrays.sort on Strings, UTF-16 order; equal to
            // code point order for BMP characters).
            keyed.sort_by(|a, b| a.0.cmp(b.0));
            self.out.push('{');
            for (i, (k, v)) in keyed.iter().enumerate() {
                if i > 0 {
                    self.out.push(',');
                }
                append_quoted(&mut self.out, k);
                self.out.push(':');
                match self.encode(*v) {
                    Ok(()) => {}
                    Err(EncErr::Msg(m)) => {
                        return Err(EncErr::Msg(format!(
                            "in {} key {}: {}",
                            x.get_type(),
                            quote(k),
                            m
                        )));
                    }
                    Err(e) => return Err(e),
                }
            }
            self.out.push('}');
            return Ok(());
        }

        // e.g. struct
        if let Some(m) = x.request_value::<StructureMarker>() {
            let mut fields = x.dir_attr();
            fields.sort();
            self.out.push('{');
            let mut first = true;
            for field in fields {
                let v = match x.get_attr(&field, self.heap) {
                    Ok(Some(v)) => v,
                    Ok(None) => continue,
                    Err(e) => {
                        return Err(EncErr::Msg(format!(
                            "in {} field .{}: {}",
                            x.get_type(),
                            field,
                            crate::error::bare_message(&e)
                        )));
                    }
                };
                // Only struct_fields of NativeInfo are emitted, not methods.
                if m.native_info && v.get_type() == "builtin_function_or_method" {
                    continue;
                }
                if !first {
                    self.out.push(',');
                }
                first = false;
                append_quoted(&mut self.out, &field);
                self.out.push(':');
                match self.encode(v) {
                    Ok(()) => {}
                    Err(EncErr::Msg(msg)) => {
                        return Err(EncErr::Msg(format!(
                            "in {} field .{}: {}",
                            x.get_type(),
                            field,
                            msg
                        )));
                    }
                    Err(e) => return Err(e),
                }
            }
            self.out.push('}');
            return Ok(());
        }

        // e.g. tuple, list, set, range
        if let Ok(it) = x.iterate(self.heap) {
            let elems: Vec<Value<'v>> = it.collect();
            self.out.push('[');
            for (i, elem) in elems.into_iter().enumerate() {
                if i > 0 {
                    self.out.push(',');
                }
                match self.encode(elem) {
                    Ok(()) => {}
                    Err(EncErr::Msg(m)) => {
                        return Err(EncErr::Msg(format!(
                            "at {} index {}: {}",
                            x.get_type(),
                            i,
                            m
                        )));
                    }
                    Err(e) => return Err(e),
                }
            }
            self.out.push(']');
            return Ok(());
        }

        Err(EncErr::Msg(format!(
            "cannot encode {} as JSON",
            json_type_name(x)
        )))
    }
}

/// Java `Starlark.type` for the "cannot encode" message (builtins are
/// `builtin_function_or_method` since starlark-rust patch 0009).
fn json_type_name(x: Value) -> String {
    x.get_type().to_owned()
}

// Java: Json.java:271 Encoder.appendQuoted
fn append_quoted(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        let cp = c as u32;
        if cp < 0x20 {
            match c {
                '\u{8}' => out.push_str("\\b"),
                '\u{c}' => out.push_str("\\f"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => {
                    out.push_str("\\u00");
                    out.push(HEX[((cp >> 4) & 0xf) as usize] as char);
                    out.push(HEX[(cp & 0xf) as usize] as char);
                }
            }
            continue;
        }
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// Java: Json.encode
pub fn json_encode<'v>(x: Value<'v>, heap: Heap<'v>) -> starlark::Result<String> {
    let mut enc = Encoder {
        out: String::new(),
        heap,
        depth: 0,
    };
    match enc.encode(x) {
        Ok(()) => Ok(enc.out),
        Err(EncErr::Msg(m)) => Err(err(m)),
        Err(EncErr::TooDeep) => Err(err("nesting depth limit exceeded".to_owned())),
    }
}

// -------------------------------------------------------------------------------------------
// Decoder

struct Decoder<'v> {
    s: Vec<char>,
    i: usize,
    heap: Heap<'v>,
    depth: usize,
}

fn isdigit(c: char) -> bool {
    c.is_ascii_digit()
}

// Java: Json.java quoteChar
fn quote_char(c: char) -> String {
    quote(&c.to_string())
}

impl<'v> Decoder<'v> {
    // Java: Json.java:382 Decoder.decode
    fn decode(&mut self) -> starlark::Result<Value<'v>> {
        match self.parse() {
            Ok(x) => {
                if self.skip_space() {
                    return Err(err(format!(
                        "at offset {}, unexpected character {} after value",
                        self.i,
                        quote_char(self.s[self.i])
                    )));
                }
                Ok(x)
            }
            Err(DecErr::TooDeep) => Err(err("nesting depth limit exceeded".to_owned())),
            Err(DecErr::Msg(m)) => Err(err(format!("at offset {}, {}", self.i, m))),
        }
    }

    // Java: Json.java:398 Decoder.parse
    fn parse(&mut self) -> Result<Value<'v>, DecErr> {
        if self.depth >= MAX_DEPTH {
            return Err(DecErr::TooDeep);
        }
        self.depth += 1;
        let r = self.parse_inner();
        self.depth -= 1;
        r
    }

    fn starts_with(&self, lit: &str) -> bool {
        let n = lit.chars().count();
        self.i + n <= self.s.len() && self.s[self.i..self.i + n].iter().copied().eq(lit.chars())
    }

    fn parse_inner(&mut self) -> Result<Value<'v>, DecErr> {
        let c = self.next()?;
        match c {
            '"' => return Ok(self.heap.alloc(self.parse_string()?)),
            'n' if self.starts_with("null") => {
                self.i += 4;
                return Ok(Value::new_none());
            }
            't' if self.starts_with("true") => {
                self.i += 4;
                return Ok(Value::new_bool(true));
            }
            'f' if self.starts_with("false") => {
                self.i += 5;
                return Ok(Value::new_bool(false));
            }
            '[' => {
                let mut list = Vec::new();
                self.i += 1; // '['
                let mut c = self.next()?;
                if c != ']' {
                    loop {
                        let elem = self.parse()?;
                        list.push(elem);
                        c = self.next()?;
                        if c != ',' {
                            if c != ']' {
                                return Err(DecErr::Msg(format!(
                                    "got {}, want ',' or ']'",
                                    quote_char(c)
                                )));
                            }
                            break;
                        }
                        self.i += 1; // ','
                    }
                }
                self.i += 1; // ']'
                return Ok(self.heap.alloc(AllocList(list)));
            }
            '{' => {
                let mut entries: Vec<(Value<'v>, Value<'v>)> = Vec::new();
                self.i += 1; // '{'
                let mut c = self.next()?;
                if c != '}' {
                    loop {
                        let key = self.parse()?;
                        if key.unpack_str().is_none() {
                            return Err(DecErr::Msg(format!(
                                "got {} for object key, want string",
                                key.get_type()
                            )));
                        }
                        c = self.next()?;
                        if c != ':' {
                            return Err(DecErr::Msg(format!(
                                "after object key, got {}, want ':' ",
                                quote_char(c)
                            )));
                        }
                        self.i += 1; // ':'
                        let value = self.parse()?;
                        // The last value for a repeated key wins, at its first position.
                        match entries
                            .iter_mut()
                            .find(|(k, _)| k.unpack_str() == key.unpack_str())
                        {
                            Some(e) => e.1 = value,
                            None => entries.push((key, value)),
                        }
                        c = self.next()?;
                        if c != ',' {
                            if c != '}' {
                                return Err(DecErr::Msg(format!(
                                    "in object, got {}, want ',' or '}}'",
                                    quote_char(c)
                                )));
                            }
                            break;
                        }
                        self.i += 1; // ','
                    }
                }
                self.i += 1; // '}'
                return Ok(self.heap.alloc(AllocDict(entries)));
            }
            c if isdigit(c) || c == '-' => return self.parse_number(),
            _ => {}
        }
        Err(DecErr::Msg(format!(
            "unexpected character {}",
            quote_char(c)
        )))
    }

    // Java: Json.java:507 Decoder.parseString
    fn parse_string(&mut self) -> Result<String, DecErr> {
        self.i += 1; // '"'
        let mut out = String::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == '"' {
                self.i += 1;
                return Ok(out);
            }
            if c != '\\' {
                if (c as u32) <= 0x1f {
                    return Err(DecErr::Msg(format!(
                        "invalid character '\\x{:02x}' in string literal",
                        c as u32
                    )));
                }
                self.i += 1;
                out.push(c);
                continue;
            }
            self.i += 1; // '\\'
            if self.i == self.s.len() {
                return Err(DecErr::Msg("incomplete escape".to_owned()));
            }
            let c = self.s[self.i];
            self.i += 1;
            match c {
                '\\' | '/' | '"' => out.push(c),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    if self.i + 4 >= self.s.len() {
                        return Err(DecErr::Msg("incomplete \\uXXXX escape".to_owned()));
                    }
                    let mut units = vec![self.parse_unicode_escape()?];
                    // Further \uXXXX escapes, so that surrogate pairs decode correctly.
                    while self.i + 6 < self.s.len()
                        && self.s[self.i] == '\\'
                        && self.s[self.i + 1] == 'u'
                    {
                        self.i += 2;
                        units.push(self.parse_unicode_escape()?);
                    }
                    // Unpaired surrogates become U+FFFD (Java's UTF-16 encoder REPLACE action).
                    for r in char::decode_utf16(units) {
                        out.push(r.unwrap_or('\u{FFFD}'));
                    }
                }
                _ => return Err(DecErr::Msg(format!("invalid escape '\\{c}'"))),
            }
        }
        Err(DecErr::Msg("unclosed string literal".to_owned()))
    }

    // Java: Json.java:581 Decoder.parseUnicodeEscape
    fn parse_unicode_escape(&mut self) -> Result<u16, DecErr> {
        let mut hex: u16 = 0;
        for j in 0..4 {
            let c = self.s[self.i + j];
            let nybble = match c {
                '0'..='9' => c as u16 - '0' as u16,
                'a'..='f' => 10 + c as u16 - 'a' as u16,
                'A'..='F' => 10 + c as u16 - 'A' as u16,
                _ => {
                    return Err(DecErr::Msg(format!(
                        "invalid hex char {} in \\uXXXX escape",
                        quote_char(c)
                    )));
                }
            };
            hex = (hex << 4) | nybble;
        }
        self.i += 4;
        Ok(hex)
    }

    // Java: Json.java:603 Decoder.parseNumber
    fn parse_number(&mut self) -> Result<Value<'v>, DecErr> {
        let mut isfloat = false;
        let mut j = self.i + 1;
        while j < self.s.len() {
            let c = self.s[j];
            if isdigit(c) {
            } else if matches!(c, '.' | 'e' | 'E' | '+' | '-') {
                isfloat = true;
            } else {
                break;
            }
            j += 1;
        }
        let num: String = self.s[self.i..j].iter().collect();
        let mut digits = self.i;
        if self.s[self.i] == '-' {
            digits += 1;
        }
        if digits == j
            || self.s[digits] == '.'
            || self.s[j - 1] == '.'
            || num.contains(".e")
            || (self.s[digits] == '0' && j - digits > 1 && isdigit(self.s[digits + 1]))
        {
            return Err(DecErr::Msg(format!("invalid number: {num}")));
        }
        self.i = j;
        if isfloat {
            // Java Double.parseDouble accepts what this scan admits, plus forms like "1e5".
            match num.parse::<f64>() {
                Ok(x) => Ok(self.heap.alloc(x)),
                Err(_) => Err(DecErr::Msg(format!("invalid number: {num}"))),
            }
        } else {
            match num.parse::<num_bigint::BigInt>() {
                Ok(x) => Ok(self.heap.alloc(x)),
                Err(_) => Err(DecErr::Msg(format!("invalid number: {num}"))),
            }
        }
    }

    // Java: Json.java skipSpace
    fn skip_space(&mut self) -> bool {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c != ' ' && c != '\t' && c != '\n' && c != '\r' {
                return true;
            }
            self.i += 1;
        }
        false
    }

    // Java: Json.java next
    fn next(&mut self) -> Result<char, DecErr> {
        if self.skip_space() {
            return Ok(self.s[self.i]);
        }
        Err(DecErr::Msg("unexpected end of file".to_owned()))
    }
}

enum DecErr {
    Msg(String),
    TooDeep,
}

/// Java: Json.decode (without `default`).
pub fn json_decode<'v>(x: &str, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
    Decoder {
        s: x.chars().collect(),
        i: 0,
        heap,
        depth: 0,
    }
    .decode()
}

// -------------------------------------------------------------------------------------------
// Indenter

/// Java: Json.java:715 indent (Indenter). Invalid input may fail or produce garbage, as in Java.
pub fn json_indent(s: &str, prefix: &str, indent: &str) -> anyhow::Result<String> {
    let s: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    // Java throws StringIndexOutOfBoundsException -> "input is not valid JSON".
    let at = |i: usize| -> anyhow::Result<char> {
        s.get(i)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("input is not valid JSON"))
    };
    let newline = |out: &mut String, depth: i64| {
        out.push('\n');
        out.push_str(prefix);
        for _ in 0..depth.max(0) {
            out.push_str(indent);
        }
    };
    let next = |i: &mut usize| -> anyhow::Result<char> {
        while *i < s.len() {
            let c = s[*i];
            if c != ' ' && c != '\t' && c != '\n' && c != '\r' {
                return Ok(c);
            }
            *i += 1;
        }
        Err(anyhow::anyhow!("unexpected end of file"))
    };
    let slice = |a: usize, b: usize| -> anyhow::Result<String> {
        if b > s.len() || a > b {
            anyhow::bail!("input is not valid JSON");
        }
        Ok(s[a..b].iter().collect())
    };
    let mut depth: i64 = 0;
    loop {
        let mut c = next(&mut i)?;
        let start = i;
        match c {
            '"' => {
                i += 1;
                c = at(i)?;
                while c != '"' {
                    if c == '\\' {
                        i += 1;
                        c = at(i)?;
                        if c == 'u' {
                            i += 4;
                        }
                    }
                    i += 1;
                    c = at(i)?;
                }
                i += 1; // '"'
                out.push_str(&slice(start, i)?);
            }
            'n' => {
                i += 4;
                out.push_str(&slice(start, i)?);
            }
            't' => {
                i += 4;
                out.push_str(&slice(start, i)?);
            }
            'f' => {
                i += 5;
                out.push_str(&slice(start, i)?);
            }
            ',' => {
                i += 1;
                out.push(',');
                newline(&mut out, depth);
            }
            '[' | '{' => {
                i += 1;
                out.push(c);
                let c2 = next(&mut i)?;
                if c2 == ']' || c2 == '}' {
                    i += 1;
                    out.push(c2);
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            ']' | '}' => {
                i += 1;
                depth -= 1;
                newline(&mut out, depth);
                out.push(c);
            }
            ':' => {
                i += 1;
                out.push_str(": ");
            }
            _ => {
                if !(isdigit(c) || c == '-') {
                    anyhow::bail!("unexpected character {}", quote_char(c));
                }
                while i < s.len() {
                    i += 1;
                    let c = at(i)?;
                    if !(isdigit(c) || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
                        break;
                    }
                }
                out.push_str(&slice(start, i)?);
            }
        }
        if depth <= 0 {
            break;
        }
    }
    Ok(out)
}

/// The `json` module (Java: `Json.INSTANCE`, put in the BUILD, .bzl and cquery environments by
/// StarlarkGlobalsImpl.addCommonUtilToplevels). Register with
/// `builder.namespace("json", register_json_members)`.
#[starlark_module]
pub fn register_json_members(builder: &mut GlobalsBuilder) {
    /// Java: Json.encode
    fn encode<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<String> {
        json_encode(x, heap)
    }

    /// Java: Json.decode
    fn decode<'v>(
        #[starlark(require = pos)] x: &str,
        // Java @Param(name = "default", named = true): also positional.
        default: Option<Value<'v>>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        match json_decode(x, heap) {
            Ok(v) => Ok(v),
            Err(e) => match default {
                Some(d) => Ok(d),
                None => Err(e),
            },
        }
    }

    /// Java: Json.indent
    fn indent<'v>(
        #[starlark(require = pos)] s: &str,
        #[starlark(require = named, default = "")] prefix: &str,
        #[starlark(require = named, default = "\t")] indent: &str,
    ) -> anyhow::Result<String> {
        json_indent(s, prefix, indent)
    }

    /// Java: Json.encodeIndent
    fn encode_indent<'v>(
        #[starlark(require = pos)] x: Value<'v>,
        #[starlark(require = named, default = "")] prefix: &str,
        #[starlark(require = named, default = "\t")] indent: &str,
        heap: Heap<'v>,
    ) -> starlark::Result<String> {
        let s = json_encode(x, heap)?;
        json_indent(&s, prefix, indent).map_err(|e| err(e.to_string()))
    }
}

/// Registers the `json` namespace.
pub fn register_json(builder: &mut GlobalsBuilder) {
    builder.namespace("json", register_json_members);
}
