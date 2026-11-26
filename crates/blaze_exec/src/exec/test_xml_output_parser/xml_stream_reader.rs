//! The part of the JDK's StAX reader that `TestXmlOutputParser` uses:
//! `XMLInputFactory.newInstance().createXMLStreamReader(InputStream)` of the oracle's JDK
//! (25.0.2; the Xerces-based `com.sun.org.apache.xerces.internal.impl.XMLStreamReaderImpl`),
//! with its defaults: namespace aware, entity references replaced, not coalescing.
//!
//! PORT: Java's reader is the JDK's Xerces scanner. This is a scanner written for the same
//! acceptance: the document's encoding (byte order mark, auto-detection, the encoding
//! declaration, Xerces' `EncodingMap`), line-end normalization, XML 1.0 character validity
//! (Xerces' `XMLChar` tables, dumped from the oracle's JDK), names and qualified names as Xerces'
//! `XMLEntityScanner.scanQName` reads them, attribute value normalization, character and
//! predefined entity references, comments, processing instructions, CDATA sections, the
//! namespace constraints of `XMLNSDocumentScannerImpl`, and where markup may appear. Each rule
//! was checked against Bazel's own `TestXmlOutputParser` on the oracle's JDK
//! (`~/.cache/blaze/work/exec-fix-1/xmlprobe/`, 145 documents; the cases are the tests of
//! `test_xml_output_parser_test.rs`). The exception messages are not Java's: Bazel drops them
//! (`TestStrategy.parseTestResult` returns null on any parse failure). One exception: the
//! malformed byte sequences the JDK's readers find (`UTF8Reader`, `ASCIIReader`, `UTF16Reader`)
//! are reported through the five-argument `XMLErrorReporter.reportError`, which the StAX error
//! reporter does not override; with no error handler set, Xerces' `DefaultErrorHandler` prints
//! `[Fatal Error] :-1:-1: <message>` to `System.err` (the console during a command) before the
//! exception is thrown. Those messages are Java's and the line is printed (review exec-2 round 2,
//! E2-11; measured on the oracle's JDK).
//!
//! The JDK 25 processing limits are ported: an element depth over 100, more than 200 attributes
//! on one element, a name longer than 1,000 characters (`jdk.xml.maxElementDepth`,
//! `jdk.xml.elementAttributeLimit`, `jdk.xml.maxXMLNameLimit`; measured on the oracle's JDK by
//! the review exec-2 round 2, E2-10).
//!
//! Not ported (`unported!`): a document type declaration with an internal subset or an external
//! identifier (Java reads the DTD and its entity declarations), XML 1.1 documents, UCS-4 and
//! EBCDIC documents, and the encodings Xerces reads through a Java `Charset` other than UTF-8,
//! UTF-16, ISO-8859-1 and US-ASCII.

use blaze_util::unported;

/// A parse failure (`XMLStreamException`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct XmlStreamException(pub(super) String);

fn error<T>(message: impl Into<String>) -> Result<T, XmlStreamException> {
    Err(XmlStreamException(message.into()))
}

/// `jdk.xml.maxElementDepth` of the oracle's JDK (25.0.2).
const MAX_ELEMENT_DEPTH: usize = 100;
/// `jdk.xml.elementAttributeLimit` of the oracle's JDK.
const ELEMENT_ATTRIBUTE_LIMIT: usize = 200;
/// `jdk.xml.maxXMLNameLimit` of the oracle's JDK.
const MAX_NAME_LIMIT: usize = 1000;

/// `XMLEntityScanner.checkLimit(Limit.MAX_NAME_LIMIT, ...)`: a name (or one part of a qualified
/// name) longer than 1,000 characters fails.
fn check_name_limit(length: usize) -> Result<(), XmlStreamException> {
    if length > MAX_NAME_LIMIT {
        return error(format!(
            "A name of {length} characters is over the limit {MAX_NAME_LIMIT} (jdk.xml.maxXMLNameLimit)."
        ));
    }
    Ok(())
}

/// A document whose bytes stop being decodable.
#[derive(Debug, Clone)]
struct DecodeError {
    message: String,
    /// The first character index whose reading fails: `None` is the end of the decoded
    /// characters (the error is seen only when the scanner reaches it, as with Xerces'
    /// `UTF8Reader`, which returns the characters before a bad sequence and fails on the next
    /// read); `Some` is the start of the chunk the JDK's reader loads with the bad byte (its
    /// `ASCIIReader` and `UTF16Reader` convert a whole chunk at once, see [`eager_chunk_start`]).
    visible_from: Option<usize>,
    /// Whether it is a reader's `MalformedByteSequenceException` (printed as `[Fatal Error]`), not
    /// a scanner error.
    reader: bool,
}

/// The size of the JDK's reader buffers (`XMLEntityManager.DEFAULT_BUFFER_SIZE`): an
/// `ASCIIReader` converts up to 8192 bytes per read, a `UTF16Reader` up to 8192 bytes (4096
/// UTF-16 units).
const READER_BUFFER_SIZE: usize = 8192;

/// The raw index (before line-end normalization; bytes for ASCII, UTF-16 units for UTF-16) of
/// the start of the chunk that holds `bad`, for a reader that converts whole chunks of
/// `chunk` units after `first` (the end of the XML declaration, or the start).
///
/// PORT: an approximation of Xerces' loads: after the declaration the scanner reads chunks of
/// the buffer's size, but a load that keeps the start of a token spanning the end of the buffer
/// (`XMLEntityScanner.load` with an offset) reads less, which moves the later boundaries; that is
/// not modelled. A bad byte right after the root element is in the first chunk after the
/// declaration (rejected, as in Java); one 200,000 characters later is never loaded (accepted, as
/// in Java).
fn eager_chunk_start(bad: usize, first: usize, chunk: usize) -> usize {
    if bad < first {
        return bad;
    }
    first + (bad - first) / chunk * chunk
}

/// The index in the normalized characters (`\r\n` and `\r` become `\n`) of the raw character
/// at `raw_index`.
fn normalized_index(raw: &[char], raw_index: usize) -> usize {
    let dropped = (1..raw_index.min(raw.len()))
        .filter(|j| raw[*j] == '\n' && raw[*j - 1] == '\r')
        .count();
    raw_index - dropped
}

/// The message of the `MalformedByteSequenceException` Xerces' `UTF8Reader` throws for the
/// sequence at `p`, the first byte that does not start a valid UTF-8 sequence (both accept the
/// same sequences; only the message depends on the kind of error).
// Java: com/sun/org/apache/xerces/internal/impl/io/UTF8Reader.java read (JDK)
fn utf8_reader_error(bytes: &[u8], p: usize) -> String {
    let invalid = |n: u32, of: u32| format!("Invalid byte {n} of {of}-byte UTF-8 sequence.");
    let expected = |n: u32, of: u32| format!("Expected byte {n} of {of}-byte UTF-8 sequence.");
    let b0 = bytes[p];
    let at = |i: usize| bytes.get(p + i).copied();
    let continuation = |b: u8| b & 0xC0 == 0x80;
    if b0 & 0xE0 == 0xC0 && b0 & 0x1E != 0 {
        // [110y yyyy] [10xx xxxx]
        return match at(1) {
            None => expected(2, 2),
            Some(_) => invalid(2, 2),
        };
    }
    if b0 & 0xF0 == 0xE0 {
        // [1110 zzzz] [10yy yyyy] [10xx xxxx]
        let Some(b1) = at(1) else {
            return expected(2, 3);
        };
        if !continuation(b1) || (b0 == 0xED && b1 >= 0xA0) || (b0 & 0x0F == 0 && b1 & 0x20 == 0) {
            return invalid(2, 3);
        }
        return match at(2) {
            None => expected(3, 3),
            Some(_) => invalid(3, 3),
        };
    }
    if b0 & 0xF8 == 0xF0 {
        // [1111 0uuu] [10uu zzzz] [10yy yyyy] [10xx xxxx]
        let Some(b1) = at(1) else {
            return expected(2, 4);
        };
        if !continuation(b1) || (b1 & 0x30 == 0 && b0 & 0x07 == 0) {
            return invalid(2, 4);
        }
        let Some(b2) = at(2) else {
            return expected(3, 4);
        };
        if !continuation(b2) {
            return invalid(3, 4);
        }
        let Some(b3) = at(3) else {
            return expected(4, 4);
        };
        if !continuation(b3) {
            return invalid(4, 4);
        }
        let uuuuu = ((b0 << 2) & 0x1C) | ((b1 >> 4) & 0x03);
        // invalidSurrogate(uuuuu): Integer.toHexString.
        return format!(
            "High surrogate bits in UTF-8 sequence must not exceed 0x10 but found 0x{uuuuu:x}."
        );
    }
    invalid(1, 1)
}

/// The events of `XMLStreamReader.next()` that `TestXmlOutputParser` distinguishes.
pub(super) enum XmlEvent {
    StartElement(StartElement),
    /// `END_ELEMENT`, with the element's local name.
    EndElement(String),
    EndDocument,
    /// Any other event (characters, comments, processing instructions, the DTD).
    Other,
}

/// A `START_ELEMENT`: its local name and its attributes (local name, value), in document order.
/// Namespace declarations are not attributes (`getAttributeCount` does not count them).
pub(super) struct StartElement {
    pub(super) local_name: String,
    pub(super) attributes: Vec<(String, String)>,
}

/// The namespace bound to the `xml` prefix.
const XML_URI: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of the `xmlns` prefix.
const XMLNS_URI: &str = "http://www.w3.org/2000/xmlns/";

/// `XMLStreamReader` over a whole document.
pub(super) struct XmlStreamReader {
    /// The decoded document after line-end normalization.
    chars: Vec<char>,
    /// Where decoding stopped, when it stopped at a malformed byte sequence (see
    /// [`DecodeError`]).
    decode_error: Option<DecodeError>,
    /// Whether the scanner reached `decode_error` and it is a reader's error (printed by `next`).
    reader_error_raised: std::cell::Cell<bool>,
    /// Whether the `[Fatal Error]` line was printed (Java throws after printing; the parser
    /// stops there).
    fatal_error_printed: bool,
    pos: usize,
    started: bool,
    /// The open elements: (raw name, local name).
    open: Vec<(String, String)>,
    /// The namespace bindings each open element declares: (prefix, URI); "" is the default.
    namespaces: Vec<Vec<(String, String)>>,
    seen_root: bool,
    root_done: bool,
    seen_doctype: bool,
    /// The end of an empty element (`<a/>`), reported by the next call.
    pending_end: bool,
}

impl XmlStreamReader {
    /// `XMLInputFactory.createXMLStreamReader(InputStream)`: detects the encoding and decodes.
    pub(super) fn new(input: &[u8]) -> Result<XmlStreamReader, XmlStreamException> {
        let (chars, decode_error) = decode(input)?;
        Ok(XmlStreamReader {
            chars,
            decode_error,
            reader_error_raised: std::cell::Cell::new(false),
            fatal_error_printed: false,
            pos: 0,
            started: false,
            open: Vec::new(),
            namespaces: Vec::new(),
            seen_root: false,
            root_done: false,
            seen_doctype: false,
            pending_end: false,
        })
    }

    /// `getLocation().getLineNumber() + ":" + getColumnNumber()` of the current position.
    pub(super) fn location(&self) -> String {
        let consumed = &self.chars[..self.pos.min(self.chars.len())];
        let line = consumed.iter().filter(|c| **c == '\n').count() + 1;
        let line_start = consumed
            .iter()
            .rposition(|c| *c == '\n')
            .map_or(0, |i| i + 1);
        format!("{}:{}", line, consumed.len() - line_start + 1)
    }

    /// `XMLStreamReader.next()`.
    pub(super) fn next(&mut self) -> Result<XmlEvent, XmlStreamException> {
        let result = self.next_event();
        if let Err(e) = &result
            && self.reader_error_raised.get()
            && !self.fatal_error_printed
        {
            self.fatal_error_printed = true;
            // XMLErrorReporter.reportError(location, domain, key, arguments, SEVERITY_FATAL_ERROR,
            // exception) with no error handler: DefaultErrorHandler.printError("Fatal Error", ex)
            // on a PrintWriter over System.err. The reader's exception has no location: line and
            // column are -1 and there is no system id.
            // Java: com/sun/org/apache/xerces/internal/util/DefaultErrorHandler.java printError (JDK)
            blaze_events::io::system_err_println(&format!("[Fatal Error] :-1:-1: {}", e.0));
        }
        result
    }

    fn next_event(&mut self) -> Result<XmlEvent, XmlStreamException> {
        if self.pending_end {
            self.pending_end = false;
            return Ok(XmlEvent::EndElement(self.close_element()));
        }
        if !self.started {
            self.started = true;
            if let Some(decl) = parse_xml_decl(&self.chars)? {
                self.pos = decl.end;
            }
        }
        if self.open.is_empty() {
            self.next_in_prolog_or_epilog()
        } else {
            self.next_in_content()
        }
    }

    /// The character at `i`, `None` at the end of the document, or the decoding error when the
    /// document's bytes stopped being decodable there.
    fn char_at(&self, i: usize) -> Result<Option<char>, XmlStreamException> {
        if let Some(e) = &self.decode_error
            && i >= e.visible_from.unwrap_or(self.chars.len())
        {
            self.reader_error_raised.set(e.reader);
            return error(e.message.clone());
        }
        Ok(self.chars.get(i).copied())
    }

    fn peek(&self) -> Result<Option<char>, XmlStreamException> {
        self.char_at(self.pos)
    }

    fn starts_with(&self, s: &str) -> Result<bool, XmlStreamException> {
        for (i, c) in s.chars().enumerate() {
            if self.char_at(self.pos + i)? != Some(c) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Skips white space (`S`); returns whether there was any.
    fn skip_spaces(&mut self) -> Result<bool, XmlStreamException> {
        let start = self.pos;
        while let Some(c) = self.peek()? {
            if !is_space(c) {
                break;
            }
            self.pos += 1;
        }
        Ok(self.pos > start)
    }

    fn expect(&mut self, s: &str) -> Result<(), XmlStreamException> {
        if !self.starts_with(s)? {
            return error(format!("Expected \"{s}\" at {}", self.location()));
        }
        self.pos += s.chars().count();
        Ok(())
    }

    /// Before the root element and after it (`Misc`, the XML declaration, the DTD).
    fn next_in_prolog_or_epilog(&mut self) -> Result<XmlEvent, XmlStreamException> {
        self.skip_spaces()?;
        let Some(c) = self.peek()? else {
            if !self.seen_root {
                return error("Premature end of file.");
            }
            return Ok(XmlEvent::EndDocument);
        };
        if c == '<' {
            if self.starts_with("<?")? {
                self.scan_pi()?;
                return Ok(XmlEvent::Other);
            }
            if self.starts_with("<!--")? {
                self.scan_comment()?;
                return Ok(XmlEvent::Other);
            }
            if self.starts_with("<!DOCTYPE")? {
                if self.seen_root || self.seen_doctype {
                    return error("Already seen doctype.");
                }
                self.scan_doctype()?;
                return Ok(XmlEvent::Other);
            }
            if let Some(n) = self.char_at(self.pos + 1)?
                && is_name_start(n)
            {
                if self.root_done {
                    return error(
                        "The markup in the document following the root element must be \
                         well-formed.",
                    );
                }
                self.pos += 1;
                return self.scan_start_element();
            }
        }
        if self.root_done {
            error("Content is not allowed in trailing section.")
        } else {
            error("Content is not allowed in prolog.")
        }
    }

    /// Inside the root element.
    fn next_in_content(&mut self) -> Result<XmlEvent, XmlStreamException> {
        let Some(c) = self.peek()? else {
            return error("XML document structures must start and end within the same entity.");
        };
        match c {
            '<' => {
                if self.starts_with("</")? {
                    self.pos += 2;
                    return self.scan_end_element();
                }
                if self.starts_with("<!--")? {
                    self.scan_comment()?;
                } else if self.starts_with("<![CDATA[")? {
                    self.scan_cdata()?;
                } else if self.starts_with("<?")? {
                    self.scan_pi()?;
                } else if self.starts_with("<!")? {
                    return error(
                        "The markup in the document preceding the root element must be well-formed.",
                    );
                } else {
                    self.pos += 1;
                    return self.scan_start_element();
                }
                Ok(XmlEvent::Other)
            }
            '&' => {
                self.scan_reference()?;
                Ok(XmlEvent::Other)
            }
            _ => {
                self.scan_char_data()?;
                Ok(XmlEvent::Other)
            }
        }
    }

    /// Character data up to the next `<` or `&`: valid characters, and no `]]>`.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLDocumentFragmentScannerImpl.java scanContent
    fn scan_char_data(&mut self) -> Result<(), XmlStreamException> {
        while let Some(c) = self.peek()? {
            if c == '<' || c == '&' {
                break;
            }
            if c == ']' && self.starts_with("]]>")? {
                return error(
                    "The character sequence \"]]>\" must not appear in content unless used to mark the end of a CDATA section.",
                );
            }
            check_valid(c, "element content")?;
            self.pos += 1;
        }
        Ok(())
    }

    /// `&...;` in content: a character reference or a predefined entity. No DTD declares other
    /// entities here (a DTD that could is `unported!`).
    fn scan_reference(&mut self) -> Result<char, XmlStreamException> {
        self.pos += 1;
        if self.peek()? == Some('#') {
            self.pos += 1;
            return self.scan_char_reference_value();
        }
        let name = self.scan_name()?;
        if self.peek()? != Some(';') {
            return error(format!(
                "The reference to entity \"{name}\" must end with the ';' delimiter."
            ));
        }
        self.pos += 1;
        match name.as_str() {
            "lt" => Ok('<'),
            "gt" => Ok('>'),
            "amp" => Ok('&'),
            "apos" => Ok('\''),
            "quot" => Ok('"'),
            _ => error(format!(
                "The entity \"{name}\" was referenced, but not declared."
            )),
        }
    }

    /// After `&#`: decimal digits, or `x` and hexadecimal digits, then `;`; the value must be a
    /// valid XML character.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLScanner.java scanCharReferenceValue
    fn scan_char_reference_value(&mut self) -> Result<char, XmlStreamException> {
        let hex = self.peek()? == Some('x');
        if hex {
            self.pos += 1;
        }
        let radix = if hex { 16 } else { 10 };
        let mut value: u32 = 0;
        let mut digits = 0;
        let mut overflow = false;
        while let Some(c) = self.peek()? {
            let Some(d) = c.to_digit(radix).filter(|_| c.is_ascii()) else {
                break;
            };
            value = value.saturating_mul(radix).saturating_add(d);
            if value > 0x10FFFF {
                overflow = true;
            }
            digits += 1;
            self.pos += 1;
        }
        if digits == 0 || self.peek()? != Some(';') {
            return error("A character reference must end with the ';' delimiter.");
        }
        self.pos += 1;
        match char::from_u32(value).filter(|c| !overflow && is_valid(*c)) {
            Some(c) => Ok(c),
            None => error("Character reference is an invalid XML character."),
        }
    }

    /// `<!-- ... -->`: no `--` inside.
    fn scan_comment(&mut self) -> Result<(), XmlStreamException> {
        self.pos += 4;
        loop {
            let Some(c) = self.peek()? else {
                return error("XML document structures must start and end within the same entity.");
            };
            if c == '-' && self.char_at(self.pos + 1)? == Some('-') {
                if self.char_at(self.pos + 2)? != Some('>') {
                    return error("The string \"--\" is not permitted within comments.");
                }
                self.pos += 3;
                return Ok(());
            }
            check_valid(c, "comment")?;
            self.pos += 1;
        }
    }

    /// `<![CDATA[ ... ]]>`.
    fn scan_cdata(&mut self) -> Result<(), XmlStreamException> {
        self.pos += 9;
        loop {
            let Some(c) = self.peek()? else {
                return error("The CDATA section must end with \"]]>\".");
            };
            if c == ']' && self.starts_with("]]>")? {
                self.pos += 3;
                return Ok(());
            }
            check_valid(c, "CDATA section")?;
            self.pos += 1;
        }
    }

    /// `<?target ...?>`: the target is a name other than `xml` in any case.
    fn scan_pi(&mut self) -> Result<(), XmlStreamException> {
        self.pos += 2;
        let target = self.scan_name()?;
        if target.eq_ignore_ascii_case("xml") {
            return error(
                "The processing instruction target matching \"[xX][mM][lL]\" is not allowed.",
            );
        }
        if self.starts_with("?>")? {
            self.pos += 2;
            return Ok(());
        }
        if !self.skip_spaces()? {
            return error(
                "White space is required between the processing instruction target and data.",
            );
        }
        loop {
            let Some(c) = self.peek()? else {
                return error("XML document structures must start and end within the same entity.");
            };
            if c == '?' && self.char_at(self.pos + 1)? == Some('>') {
                self.pos += 2;
                return Ok(());
            }
            check_valid(c, "processing instruction")?;
            self.pos += 1;
        }
    }

    /// `<!DOCTYPE name>`. A DTD (internal subset or external identifier) can declare entities
    /// and attribute defaults, which Java reads; not ported.
    fn scan_doctype(&mut self) -> Result<(), XmlStreamException> {
        self.pos += 9;
        if !self.skip_spaces()? {
            return error(
                "White space is required after \"<!DOCTYPE\" in the document type declaration.",
            );
        }
        self.scan_name()?;
        self.skip_spaces()?;
        match self.peek()? {
            Some('>') => {
                self.pos += 1;
                self.seen_doctype = true;
                Ok(())
            }
            Some('[') | Some('S') | Some('P') => unported!(
                "TestXmlOutputParser.parseXmlToTree",
                "a test.xml with a document type definition (StAX reads its declarations)"
            ),
            _ => error("The document type declaration must end with '>'."),
        }
    }

    /// A `Name`.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLEntityScanner.java scanName
    fn scan_name(&mut self) -> Result<String, XmlStreamException> {
        match self.peek()? {
            Some(c) if is_name_start(c) => {}
            _ => return error(format!("A name was expected at {}.", self.location())),
        }
        let start = self.pos;
        self.pos += 1;
        while let Some(c) = self.peek()? {
            if !is_name(c) {
                break;
            }
            self.pos += 1;
        }
        check_name_limit(self.pos - start)?;
        Ok(self.chars[start..self.pos].iter().collect())
    }

    /// A qualified name: (raw name, prefix, local part). The first character is not looked at
    /// for a colon (a leading colon makes no prefix); a second colon ends the name; the local
    /// part must start with an `NCNameStart` character.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLEntityScanner.java scanQName
    fn scan_qname(&mut self) -> Result<(String, Option<String>, String), XmlStreamException> {
        match self.peek()? {
            Some(c) if is_name_start(c) => {}
            _ => return error(format!("A name was expected at {}.", self.location())),
        }
        let offset = self.pos;
        self.pos += 1;
        let mut index: Option<usize> = None;
        while let Some(c) = self.peek()? {
            if !is_name(c) {
                break;
            }
            if c == ':' {
                if index.is_some() {
                    break;
                }
                index = Some(self.pos);
            }
            self.pos += 1;
        }
        // The limit holds for the prefix and the local part each (measured: a 1000-character
        // prefix with a 1000-character local part passes, a 1001-character part fails).
        match index {
            None => check_name_limit(self.pos - offset)?,
            Some(index) => {
                check_name_limit(index - offset)?;
                check_name_limit(self.pos - index - 1)?;
            }
        }
        let raw: String = self.chars[offset..self.pos].iter().collect();
        match index {
            None => Ok((raw.clone(), None, raw)),
            Some(index) => {
                let start_local = index + 1;
                if !self.char_at(start_local)?.is_some_and(is_nc_name_start) {
                    return error(format!(
                        "Element or attribute \"{raw}\" do not match QName production: QName::=(NCName':')?NCName."
                    ));
                }
                let prefix: String = self.chars[offset..index].iter().collect();
                let local: String = self.chars[start_local..self.pos].iter().collect();
                Ok((raw, Some(prefix), local))
            }
        }
    }

    /// After `<`: the element's name, its attributes, `>` or `/>`; then the namespace
    /// constraints.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLNSDocumentScannerImpl.java scanStartElement
    fn scan_start_element(&mut self) -> Result<XmlEvent, XmlStreamException> {
        let (raw, prefix, local) = self.scan_qname()?;
        // jdk.xml.maxElementDepth (JDK 25 default 100): the 101st nested element fails.
        // Java: com/sun/org/apache/xerces/internal/impl/XMLDocumentFragmentScannerImpl.java scanStartElement (JDK)
        if self.open.len() + 1 > MAX_ELEMENT_DEPTH {
            return error(format!(
                "The element \"{raw}\" has a depth of {} over the limit {MAX_ELEMENT_DEPTH} (jdk.xml.maxElementDepth).",
                self.open.len() + 1
            ));
        }
        // (raw name, prefix, local part, value)
        let mut attributes: Vec<(String, Option<String>, String, String)> = Vec::new();
        let empty = loop {
            let had_space = self.skip_spaces()?;
            match self.peek()? {
                Some('>') => {
                    self.pos += 1;
                    break false;
                }
                Some('/') => {
                    if self.char_at(self.pos + 1)? != Some('>') {
                        return error(format!(
                            "Element type \"{raw}\" must be followed by either attribute specifications, \">\" or \"/>\"."
                        ));
                    }
                    self.pos += 2;
                    break true;
                }
                Some(_) if had_space => {
                    let (attr_raw, attr_prefix, attr_local) = self.scan_qname()?;
                    if attributes.iter().any(|a| a.0 == attr_raw) {
                        return error(format!(
                            "Attribute \"{attr_raw}\" was already specified for element \"{raw}\"."
                        ));
                    }
                    self.skip_spaces()?;
                    self.expect("=")?;
                    self.skip_spaces()?;
                    let value = self.scan_attribute_value()?;
                    attributes.push((attr_raw, attr_prefix, attr_local, value));
                    // jdk.xml.elementAttributeLimit (JDK 25 default 200). Namespace declarations
                    // do not count (measured: one declaration and 200 attributes pass).
                    // Java: com/sun/org/apache/xerces/internal/impl/XMLNSDocumentScannerImpl.java scanStartElement (JDK)
                    let counted = attributes
                        .iter()
                        .filter(|a| a.0 != "xmlns" && a.1.as_deref() != Some("xmlns"))
                        .count();
                    if counted > ELEMENT_ATTRIBUTE_LIMIT {
                        return error(format!(
                            "Element \"{raw}\" has more than {ELEMENT_ATTRIBUTE_LIMIT} attributes (jdk.xml.elementAttributeLimit)."
                        ));
                    }
                }
                Some(_) => {
                    return error(format!(
                        "Element type \"{raw}\" must be followed by either attribute specifications, \">\" or \"/>\"."
                    ));
                }
                None => {
                    return error(
                        "XML document structures must start and end within the same entity.",
                    );
                }
            }
        };

        // Namespace declarations of this element.
        let mut bindings: Vec<(String, String)> = Vec::new();
        for (attr_raw, attr_prefix, attr_local, value) in &attributes {
            if attr_raw == "xmlns" {
                if value == XML_URI || value == XMLNS_URI {
                    return error(format!(
                        "The namespace \"{value}\" cannot be bound as the default namespace."
                    ));
                }
                bindings.push((String::new(), value.clone()));
            } else if attr_prefix.as_deref() == Some("xmlns") {
                if attr_local == "xmlns" {
                    return error(
                        "The prefix \"xmlns\" cannot be bound to any namespace explicitly.",
                    );
                }
                if attr_local == "xml" {
                    if value != XML_URI {
                        return error(
                            "The prefix \"xml\" cannot be bound to any namespace other than its usual namespace.",
                        );
                    }
                } else if value == XML_URI || value == XMLNS_URI {
                    return error(format!(
                        "The namespace \"{value}\" can only be bound to its usual prefix."
                    ));
                }
                if value.is_empty() {
                    return error(format!(
                        "The value of the attribute \"{attr_raw}\" is invalid. Prefixed namespace bindings may not be empty."
                    ));
                }
                bindings.push((attr_local.clone(), value.clone()));
            }
        }
        self.namespaces.push(bindings);
        self.open.push((raw.clone(), local.clone()));
        self.seen_root = true;

        if let Some(prefix) = &prefix
            && (prefix == "xmlns" || self.lookup_namespace(prefix).is_none())
        {
            return error(format!(
                "The prefix \"{prefix}\" for element \"{raw}\" is not bound."
            ));
        }
        let mut expanded: Vec<(String, String)> = Vec::new();
        let mut result = Vec::new();
        for (attr_raw, attr_prefix, attr_local, value) in attributes {
            if attr_raw == "xmlns" || attr_prefix.as_deref() == Some("xmlns") {
                continue;
            }
            if let Some(attr_prefix) = &attr_prefix {
                let Some(uri) = self.lookup_namespace(attr_prefix) else {
                    return error(format!(
                        "The prefix \"{attr_prefix}\" for attribute \"{attr_raw}\" associated with an element type \"{raw}\" is not bound."
                    ));
                };
                if expanded.iter().any(|(u, l)| *u == uri && *l == attr_local) {
                    return error(format!(
                        "Attribute \"{attr_local}\" bound to namespace \"{uri}\" was already specified for element \"{raw}\"."
                    ));
                }
                expanded.push((uri, attr_local.clone()));
            }
            result.push((attr_local, value));
        }
        self.pending_end = empty;
        Ok(XmlEvent::StartElement(StartElement {
            local_name: local,
            attributes: result,
        }))
    }

    /// The namespace bound to `prefix` in the open elements (`xml` is always bound).
    fn lookup_namespace(&self, prefix: &str) -> Option<String> {
        if prefix == "xml" {
            return Some(XML_URI.to_string());
        }
        for scope in self.namespaces.iter().rev() {
            if let Some((_, uri)) = scope.iter().rev().find(|(p, _)| p == prefix) {
                return Some(uri.clone());
            }
        }
        None
    }

    /// A quoted attribute value, normalized: references replaced, each white space character
    /// (after line-end normalization) a space; `<` is not allowed.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLScanner.java scanAttributeValue
    fn scan_attribute_value(&mut self) -> Result<String, XmlStreamException> {
        let quote = match self.peek()? {
            Some(q @ ('"' | '\'')) => q,
            _ => return error("Open quote is expected for attribute."),
        };
        self.pos += 1;
        let mut value = String::new();
        loop {
            let Some(c) = self.peek()? else {
                return error("XML document structures must start and end within the same entity.");
            };
            if c == quote {
                self.pos += 1;
                return Ok(value);
            }
            match c {
                '<' => {
                    return error("The value of attribute must not contain the '<' character.");
                }
                '&' => value.push(self.scan_reference()?),
                '\t' | '\n' | '\r' => {
                    value.push(' ');
                    self.pos += 1;
                }
                _ => {
                    check_valid(c, "attribute value")?;
                    value.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// After `</`: the open element's raw name, white space, `>`.
    // Java: com/sun/org/apache/xerces/internal/impl/XMLDocumentFragmentScannerImpl.java scanEndElement
    fn scan_end_element(&mut self) -> Result<XmlEvent, XmlStreamException> {
        let (raw, _) = self.open.last().expect("an open element").clone();
        if !self.starts_with(&raw)? {
            return error(format!(
                "The element type \"{raw}\" must be terminated by the matching end-tag \"</{raw}>\"."
            ));
        }
        self.pos += raw.chars().count();
        self.skip_spaces()?;
        if self.peek()? != Some('>') {
            return error(format!(
                "The end-tag for element type \"{raw}\" must end with a '>' delimiter."
            ));
        }
        self.pos += 1;
        Ok(XmlEvent::EndElement(self.close_element()))
    }

    fn close_element(&mut self) -> String {
        let (_, local) = self.open.pop().expect("an open element");
        self.namespaces.pop();
        if self.open.is_empty() {
            self.root_done = true;
        }
        local
    }
}

/// The XML declaration: where it ends, and its encoding.
struct XmlDecl {
    end: usize,
    encoding: Option<String>,
}

/// The XML declaration at the start of `chars`, if any: `<?xml` S `version` [`encoding`]
/// [`standalone`] S? `?>`.
// Java: com/sun/org/apache/xerces/internal/impl/XMLDocumentFragmentScannerImpl.java scanXMLDeclOrTextDecl
fn parse_xml_decl(chars: &[char]) -> Result<Option<XmlDecl>, XmlStreamException> {
    let starts = |pos: usize, s: &str| {
        s.chars()
            .enumerate()
            .all(|(i, c)| chars.get(pos + i) == Some(&c))
    };
    if !(starts(0, "<?xml") && chars.get(5).is_some_and(|c| is_space(*c))) {
        return Ok(None);
    }
    let mut pos = 5;
    let skip_spaces = |pos: &mut usize| {
        let start = *pos;
        while chars.get(*pos).is_some_and(|c| is_space(*c)) {
            *pos += 1;
        }
        *pos > start
    };
    // name S? = S? quoted value
    let pseudo_attribute = |pos: &mut usize, name: &str| -> Result<String, XmlStreamException> {
        *pos += name.len();
        skip_spaces(pos);
        if chars.get(*pos) != Some(&'=') {
            return error(format!(
                "The ' = ' character must follow \"{name}\" in the XML declaration."
            ));
        }
        *pos += 1;
        skip_spaces(pos);
        let quote = match chars.get(*pos) {
            Some(q @ ('"' | '\'')) => *q,
            _ => {
                return error(format!(
                    "The value following \"{name}\" in the XML declaration must be a quoted string."
                ));
            }
        };
        *pos += 1;
        let start = *pos;
        while let Some(c) = chars.get(*pos) {
            if *c == quote {
                let value: String = chars[start..*pos].iter().collect();
                *pos += 1;
                return Ok(value);
            }
            *pos += 1;
        }
        error("XML document structures must start and end within the same entity.")
    };
    skip_spaces(&mut pos);
    if !starts(pos, "version") {
        return error("The version is required in the XML declaration.");
    }
    let version = pseudo_attribute(&mut pos, "version")?;
    match version.as_str() {
        "1.0" => {}
        "1.1" => unported!(
            "TestXmlOutputParser.parseXmlToTree",
            "an XML 1.1 test.xml (StAX applies the XML 1.1 character and name rules)"
        ),
        _ => {
            return error(format!(
                "XML version \"{version}\" is not supported, only XML 1.0 is supported."
            ));
        }
    }
    let mut encoding = None;
    let mut had_space = skip_spaces(&mut pos);
    if starts(pos, "encoding") {
        if !had_space {
            return error(
                "White space is required before the encoding pseudo attribute in the XML declaration.",
            );
        }
        let name = pseudo_attribute(&mut pos, "encoding")?;
        // EncName ::= [A-Za-z] ([A-Za-z0-9._] | '-')*
        let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
        if !valid {
            return error(format!("Invalid encoding name \"{name}\"."));
        }
        encoding = Some(name);
        had_space = skip_spaces(&mut pos);
    }
    if starts(pos, "standalone") {
        if !had_space {
            return error(
                "White space is required before the standalone pseudo attribute in the XML declaration.",
            );
        }
        let standalone = pseudo_attribute(&mut pos, "standalone")?;
        if standalone != "yes" && standalone != "no" {
            return error(format!(
                "The standalone document declaration value must be \"yes\" or \"no\", not \"{standalone}\"."
            ));
        }
        skip_spaces(&mut pos);
    }
    if !starts(pos, "?>") {
        return error("The XML declaration must end with \"?>\".");
    }
    Ok(Some(XmlDecl {
        end: pos + 2,
        encoding,
    }))
}

/// The encodings this port reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Utf8,
    Utf16Be,
    Utf16Le,
    Latin1,
    Ascii,
}

/// Detects the encoding (`XMLEntityManager.getEncodingName`, then the encoding declaration as
/// `XMLEntityScanner.setEncoding` applies it) and decodes the document, normalizing line ends.
/// Returns the characters decoded before the first malformed sequence and the error there.
// Java: com/sun/org/apache/xerces/internal/impl/XMLEntityManager.java XMLEntityManager.setupCurrentEntity
// Java: com/sun/org/apache/xerces/internal/impl/XMLEntityManager.java XMLEntityManager.createReader
fn decode(input: &[u8]) -> Result<(Vec<char>, Option<DecodeError>), XmlStreamException> {
    let (detected, skip) = match input {
        [0xFE, 0xFF, ..] => (Encoding::Utf16Be, 2),
        [0xFF, 0xFE, ..] => (Encoding::Utf16Le, 2),
        [0xEF, 0xBB, 0xBF, ..] => (Encoding::Utf8, 3),
        [0x00, 0x00, 0x00, 0x3C, ..]
        | [0x3C, 0x00, 0x00, 0x00, ..]
        | [0x00, 0x00, 0x3C, 0x00, ..]
        | [0x00, 0x3C, 0x00, 0x00, ..] => {
            unported!("TestXmlOutputParser.parseXmlToTree", "a UCS-4 test.xml")
        }
        [0x00, 0x3C, 0x00, 0x3F, ..] => (Encoding::Utf16Be, 0),
        [0x3C, 0x00, 0x3F, 0x00, ..] => (Encoding::Utf16Le, 0),
        [0x4C, 0x6F, 0xA7, 0x94, ..] => {
            unported!("TestXmlOutputParser.parseXmlToTree", "an EBCDIC test.xml")
        }
        _ => (Encoding::Utf8, 0),
    };
    let bytes = &input[skip..];
    if detected == Encoding::Utf16Be || detected == Encoding::Utf16Le {
        let decoded = decode_utf16(bytes, detected == Encoding::Utf16Be);
        // UTF-16 is a special case: a declared "UTF-16", or the detected byte order, keeps the
        // reader; any other declared encoding switches to a reader that cannot read on.
        if let Some(encoding) = parse_xml_decl(&decoded.0)?.and_then(|d| d.encoding) {
            let upper = encoding.to_ascii_uppercase();
            let detected_name = if detected == Encoding::Utf16Be {
                "UTF-16BE"
            } else {
                "UTF-16LE"
            };
            if upper != "UTF-16" && upper != detected_name {
                return error(format!(
                    "Invalid encoding \"{encoding}\" declared in a UTF-16 document."
                ));
            }
        }
        return Ok(decoded);
    }
    // An ASCII-compatible stream: the declaration (read as ASCII) names the encoding.
    let head: Vec<char> = bytes
        .iter()
        .take_while(|b| **b != b'>')
        .map(|b| *b as char)
        .chain(std::iter::once('>'))
        .collect();
    let head_decl = parse_xml_decl(&head)?;
    let decl_end = head_decl.as_ref().map_or(0, |d| d.end);
    let encoding = match head_decl.and_then(|d| d.encoding) {
        None => Encoding::Utf8,
        Some(name) => {
            let upper = name.to_ascii_uppercase();
            if upper == "UTF-8" {
                Encoding::Utf8
            } else if LATIN1_NAMES.contains(&upper.as_str()) {
                Encoding::Latin1
            } else if ASCII_NAMES.contains(&upper.as_str()) {
                Encoding::Ascii
            } else if upper == "UTF-16" || upper == "UTF-16BE" || upper == "UTF-16LE" {
                return error(format!(
                    "Invalid encoding \"{name}\" declared in a document that is not UTF-16."
                ));
            } else if upper == "ISO-10646-UCS-4"
                || upper == "ISO-10646-UCS-2"
                || OTHER_SUPPORTED_NAMES.contains(&upper.as_str())
            {
                unported!(
                    "TestXmlOutputParser.parseXmlToTree",
                    "a test.xml in encoding {name} (read through a Java Charset)"
                )
            } else {
                return error(format!("Invalid encoding name \"{name}\"."));
            }
        }
    };
    let mut chars = Vec::with_capacity(bytes.len());
    let decode_error = match encoding {
        Encoding::Latin1 => {
            chars.extend(bytes.iter().map(|b| *b as char));
            None
        }
        Encoding::Ascii => {
            // Java: com/sun/org/apache/xerces/internal/impl/io/ASCIIReader.java read (JDK)
            let bad = bytes.iter().position(|b| *b >= 0x80);
            let end = bad.unwrap_or(bytes.len());
            chars.extend(bytes[..end].iter().map(|b| *b as char));
            bad.map(|i| DecodeError {
                message: format!(
                    "Byte \"{}\" is not a member of the (7-bit) ASCII character set.",
                    bytes[i]
                ),
                visible_from: Some(normalized_index(
                    &chars,
                    eager_chunk_start(i, decl_end, READER_BUFFER_SIZE),
                )),
                reader: true,
            })
        }
        _ => match std::str::from_utf8(bytes) {
            Ok(s) => {
                chars.extend(s.chars());
                None
            }
            Err(e) => {
                let valid = e.valid_up_to();
                chars.extend(
                    std::str::from_utf8(&bytes[..valid])
                        .expect("valid prefix")
                        .chars(),
                );
                // Xerces' UTF8Reader rejects CESU-8 (surrogates encoded on their own) as Rust
                // does: `ED A0`..`ED BF` is "Invalid byte 2 of 3-byte UTF-8 sequence.".
                Some(DecodeError {
                    message: utf8_reader_error(bytes, valid),
                    visible_from: None,
                    reader: true,
                })
            }
        },
    };
    Ok((normalize_line_ends(chars), decode_error))
}

/// Decodes UTF-16. Xerces' `UTF16Reader` does not check surrogates (an unpaired one is an
/// invalid XML character for the scanner, seen when it is reached, and not printed); an odd byte
/// at the end is its `Expected byte 2 of 2-byte UTF-8 sequence.`, thrown for the whole chunk.
// Java: com/sun/org/apache/xerces/internal/impl/io/UTF16Reader.java read (JDK)
fn decode_utf16(bytes: &[u8], big_endian: bool) -> (Vec<char>, Option<DecodeError>) {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| {
            if big_endian {
                u16::from_be_bytes([pair[0], pair[1]])
            } else {
                u16::from_le_bytes([pair[0], pair[1]])
            }
        })
        .collect();
    let mut chars = Vec::with_capacity(units.len());
    let mut decode_error = None;
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(c) => chars.push(c),
            Err(_) => {
                decode_error = Some(DecodeError {
                    message: "Invalid UTF-16 surrogate.".to_string(),
                    visible_from: None,
                    reader: false,
                });
                break;
            }
        }
    }
    if decode_error.is_none() && units.len() * 2 < bytes.len() {
        // The end of the XML declaration in units (the declaration is ASCII: one unit per char).
        let decl_end = parse_xml_decl(&chars).ok().flatten().map_or(0, |d| d.end);
        let chunk_start_unit = eager_chunk_start(units.len(), decl_end, READER_BUFFER_SIZE / 2);
        // Units to characters: a supplementary character is two units.
        let mut unit = 0;
        let mut raw_char_index = chars.len();
        for (i, c) in chars.iter().enumerate() {
            if unit >= chunk_start_unit {
                raw_char_index = i;
                break;
            }
            unit += c.len_utf16();
        }
        decode_error = Some(DecodeError {
            message: "Expected byte 2 of 2-byte UTF-8 sequence.".to_string(),
            visible_from: Some(normalized_index(&chars, raw_char_index)),
            reader: true,
        });
    }
    (normalize_line_ends(chars), decode_error)
}

/// `\r\n` and a lone `\r` become `\n`.
fn normalize_line_ends(chars: Vec<char>) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\r' {
            out.push('\n');
            if chars.get(i + 1) == Some(&'\n') {
                i += 1;
            }
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

/// `XMLChar.isSpace`.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// `XMLChar.isValid` (a supplementary character is valid).
fn is_valid(c: char) -> bool {
    matches!(c as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn check_valid(c: char, place: &str) -> Result<(), XmlStreamException> {
    if is_valid(c) {
        Ok(())
    } else {
        error(format!(
            "An invalid XML character (Unicode: 0x{:x}) was found in the {place}.",
            c as u32
        ))
    }
}

fn in_table(c: char, table: &[(u16, u16)]) -> bool {
    let Ok(c) = u16::try_from(c as u32) else {
        // Supplementary characters are not name characters in XML 1.0.
        return false;
    };
    table
        .binary_search_by(|(lo, hi)| {
            if *hi < c {
                std::cmp::Ordering::Less
            } else if *lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// `XMLChar.isNameStart`.
fn is_name_start(c: char) -> bool {
    in_table(c, NAME_START)
}

/// `XMLChar.isName`.
fn is_name(c: char) -> bool {
    in_table(c, NAME)
}

/// `XMLChar.isNCNameStart`: a name start character other than `:`.
fn is_nc_name_start(c: char) -> bool {
    c != ':' && is_name_start(c)
}

// The tables of `com.sun.org.apache.xerces.internal.util.XMLChar` (XML 1.0 second edition) of
// the oracle's JDK, dumped with `xmlprobe/src/XmlCharDump.java`: BMP ranges, inclusive.
// Java: com/sun/org/apache/xerces/internal/util/XMLChar.java XMLChar
pub(crate) const NAME_START: &[(u16, u16)] = &[
    (0x003A, 0x003A),
    (0x0041, 0x005A),
    (0x005F, 0x005F),
    (0x0061, 0x007A),
    (0x00C0, 0x00D6),
    (0x00D8, 0x00F6),
    (0x00F8, 0x0131),
    (0x0134, 0x013E),
    (0x0141, 0x0148),
    (0x014A, 0x017E),
    (0x0180, 0x01C3),
    (0x01CD, 0x01F0),
    (0x01F4, 0x01F5),
    (0x01FA, 0x0217),
    (0x0250, 0x02A8),
    (0x02BB, 0x02C1),
    (0x0386, 0x0386),
    (0x0388, 0x038A),
    (0x038C, 0x038C),
    (0x038E, 0x03A1),
    (0x03A3, 0x03CE),
    (0x03D0, 0x03D6),
    (0x03DA, 0x03DA),
    (0x03DC, 0x03DC),
    (0x03DE, 0x03DE),
    (0x03E0, 0x03E0),
    (0x03E2, 0x03F3),
    (0x0401, 0x040C),
    (0x040E, 0x044F),
    (0x0451, 0x045C),
    (0x045E, 0x0481),
    (0x0490, 0x04C4),
    (0x04C7, 0x04C8),
    (0x04CB, 0x04CC),
    (0x04D0, 0x04EB),
    (0x04EE, 0x04F5),
    (0x04F8, 0x04F9),
    (0x0531, 0x0556),
    (0x0559, 0x0559),
    (0x0561, 0x0586),
    (0x05D0, 0x05EA),
    (0x05F0, 0x05F2),
    (0x0621, 0x063A),
    (0x0641, 0x064A),
    (0x0671, 0x06B7),
    (0x06BA, 0x06BE),
    (0x06C0, 0x06CE),
    (0x06D0, 0x06D3),
    (0x06D5, 0x06D5),
    (0x06E5, 0x06E6),
    (0x0905, 0x0939),
    (0x093D, 0x093D),
    (0x0958, 0x0961),
    (0x0985, 0x098C),
    (0x098F, 0x0990),
    (0x0993, 0x09A8),
    (0x09AA, 0x09B0),
    (0x09B2, 0x09B2),
    (0x09B6, 0x09B9),
    (0x09DC, 0x09DD),
    (0x09DF, 0x09E1),
    (0x09F0, 0x09F1),
    (0x0A05, 0x0A0A),
    (0x0A0F, 0x0A10),
    (0x0A13, 0x0A28),
    (0x0A2A, 0x0A30),
    (0x0A32, 0x0A33),
    (0x0A35, 0x0A36),
    (0x0A38, 0x0A39),
    (0x0A59, 0x0A5C),
    (0x0A5E, 0x0A5E),
    (0x0A72, 0x0A74),
    (0x0A85, 0x0A8B),
    (0x0A8D, 0x0A8D),
    (0x0A8F, 0x0A91),
    (0x0A93, 0x0AA8),
    (0x0AAA, 0x0AB0),
    (0x0AB2, 0x0AB3),
    (0x0AB5, 0x0AB9),
    (0x0ABD, 0x0ABD),
    (0x0AE0, 0x0AE0),
    (0x0B05, 0x0B0C),
    (0x0B0F, 0x0B10),
    (0x0B13, 0x0B28),
    (0x0B2A, 0x0B30),
    (0x0B32, 0x0B33),
    (0x0B36, 0x0B39),
    (0x0B3D, 0x0B3D),
    (0x0B5C, 0x0B5D),
    (0x0B5F, 0x0B61),
    (0x0B85, 0x0B8A),
    (0x0B8E, 0x0B90),
    (0x0B92, 0x0B95),
    (0x0B99, 0x0B9A),
    (0x0B9C, 0x0B9C),
    (0x0B9E, 0x0B9F),
    (0x0BA3, 0x0BA4),
    (0x0BA8, 0x0BAA),
    (0x0BAE, 0x0BB5),
    (0x0BB7, 0x0BB9),
    (0x0C05, 0x0C0C),
    (0x0C0E, 0x0C10),
    (0x0C12, 0x0C28),
    (0x0C2A, 0x0C33),
    (0x0C35, 0x0C39),
    (0x0C60, 0x0C61),
    (0x0C85, 0x0C8C),
    (0x0C8E, 0x0C90),
    (0x0C92, 0x0CA8),
    (0x0CAA, 0x0CB3),
    (0x0CB5, 0x0CB9),
    (0x0CDE, 0x0CDE),
    (0x0CE0, 0x0CE1),
    (0x0D05, 0x0D0C),
    (0x0D0E, 0x0D10),
    (0x0D12, 0x0D28),
    (0x0D2A, 0x0D39),
    (0x0D60, 0x0D61),
    (0x0E01, 0x0E2E),
    (0x0E30, 0x0E30),
    (0x0E32, 0x0E33),
    (0x0E40, 0x0E45),
    (0x0E81, 0x0E82),
    (0x0E84, 0x0E84),
    (0x0E87, 0x0E88),
    (0x0E8A, 0x0E8A),
    (0x0E8D, 0x0E8D),
    (0x0E94, 0x0E97),
    (0x0E99, 0x0E9F),
    (0x0EA1, 0x0EA3),
    (0x0EA5, 0x0EA5),
    (0x0EA7, 0x0EA7),
    (0x0EAA, 0x0EAB),
    (0x0EAD, 0x0EAE),
    (0x0EB0, 0x0EB0),
    (0x0EB2, 0x0EB3),
    (0x0EBD, 0x0EBD),
    (0x0EC0, 0x0EC4),
    (0x0F40, 0x0F47),
    (0x0F49, 0x0F69),
    (0x10A0, 0x10C5),
    (0x10D0, 0x10F6),
    (0x1100, 0x1100),
    (0x1102, 0x1103),
    (0x1105, 0x1107),
    (0x1109, 0x1109),
    (0x110B, 0x110C),
    (0x110E, 0x1112),
    (0x113C, 0x113C),
    (0x113E, 0x113E),
    (0x1140, 0x1140),
    (0x114C, 0x114C),
    (0x114E, 0x114E),
    (0x1150, 0x1150),
    (0x1154, 0x1155),
    (0x1159, 0x1159),
    (0x115F, 0x1161),
    (0x1163, 0x1163),
    (0x1165, 0x1165),
    (0x1167, 0x1167),
    (0x1169, 0x1169),
    (0x116D, 0x116E),
    (0x1172, 0x1173),
    (0x1175, 0x1175),
    (0x119E, 0x119E),
    (0x11A8, 0x11A8),
    (0x11AB, 0x11AB),
    (0x11AE, 0x11AF),
    (0x11B7, 0x11B8),
    (0x11BA, 0x11BA),
    (0x11BC, 0x11C2),
    (0x11EB, 0x11EB),
    (0x11F0, 0x11F0),
    (0x11F9, 0x11F9),
    (0x1E00, 0x1E9B),
    (0x1EA0, 0x1EF9),
    (0x1F00, 0x1F15),
    (0x1F18, 0x1F1D),
    (0x1F20, 0x1F45),
    (0x1F48, 0x1F4D),
    (0x1F50, 0x1F57),
    (0x1F59, 0x1F59),
    (0x1F5B, 0x1F5B),
    (0x1F5D, 0x1F5D),
    (0x1F5F, 0x1F7D),
    (0x1F80, 0x1FB4),
    (0x1FB6, 0x1FBC),
    (0x1FBE, 0x1FBE),
    (0x1FC2, 0x1FC4),
    (0x1FC6, 0x1FCC),
    (0x1FD0, 0x1FD3),
    (0x1FD6, 0x1FDB),
    (0x1FE0, 0x1FEC),
    (0x1FF2, 0x1FF4),
    (0x1FF6, 0x1FFC),
    (0x2126, 0x2126),
    (0x212A, 0x212B),
    (0x212E, 0x212E),
    (0x2180, 0x2182),
    (0x3007, 0x3007),
    (0x3021, 0x3029),
    (0x3041, 0x3094),
    (0x30A1, 0x30FA),
    (0x3105, 0x312C),
    (0x4E00, 0x9FA5),
    (0xAC00, 0xD7A3),
];

pub(crate) const NAME: &[(u16, u16)] = &[
    (0x002D, 0x002E),
    (0x0030, 0x003A),
    (0x0041, 0x005A),
    (0x005F, 0x005F),
    (0x0061, 0x007A),
    (0x00B7, 0x00B7),
    (0x00C0, 0x00D6),
    (0x00D8, 0x00F6),
    (0x00F8, 0x0131),
    (0x0134, 0x013E),
    (0x0141, 0x0148),
    (0x014A, 0x017E),
    (0x0180, 0x01C3),
    (0x01CD, 0x01F0),
    (0x01F4, 0x01F5),
    (0x01FA, 0x0217),
    (0x0250, 0x02A8),
    (0x02BB, 0x02C1),
    (0x02D0, 0x02D1),
    (0x0300, 0x0345),
    (0x0360, 0x0361),
    (0x0386, 0x038A),
    (0x038C, 0x038C),
    (0x038E, 0x03A1),
    (0x03A3, 0x03CE),
    (0x03D0, 0x03D6),
    (0x03DA, 0x03DA),
    (0x03DC, 0x03DC),
    (0x03DE, 0x03DE),
    (0x03E0, 0x03E0),
    (0x03E2, 0x03F3),
    (0x0401, 0x040C),
    (0x040E, 0x044F),
    (0x0451, 0x045C),
    (0x045E, 0x0481),
    (0x0483, 0x0486),
    (0x0490, 0x04C4),
    (0x04C7, 0x04C8),
    (0x04CB, 0x04CC),
    (0x04D0, 0x04EB),
    (0x04EE, 0x04F5),
    (0x04F8, 0x04F9),
    (0x0531, 0x0556),
    (0x0559, 0x0559),
    (0x0561, 0x0586),
    (0x0591, 0x05A1),
    (0x05A3, 0x05B9),
    (0x05BB, 0x05BD),
    (0x05BF, 0x05BF),
    (0x05C1, 0x05C2),
    (0x05C4, 0x05C4),
    (0x05D0, 0x05EA),
    (0x05F0, 0x05F2),
    (0x0621, 0x063A),
    (0x0640, 0x0652),
    (0x0660, 0x0669),
    (0x0670, 0x06B7),
    (0x06BA, 0x06BE),
    (0x06C0, 0x06CE),
    (0x06D0, 0x06D3),
    (0x06D5, 0x06E8),
    (0x06EA, 0x06ED),
    (0x06F0, 0x06F9),
    (0x0901, 0x0903),
    (0x0905, 0x0939),
    (0x093C, 0x094D),
    (0x0951, 0x0954),
    (0x0958, 0x0963),
    (0x0966, 0x096F),
    (0x0981, 0x0983),
    (0x0985, 0x098C),
    (0x098F, 0x0990),
    (0x0993, 0x09A8),
    (0x09AA, 0x09B0),
    (0x09B2, 0x09B2),
    (0x09B6, 0x09B9),
    (0x09BC, 0x09BC),
    (0x09BE, 0x09C4),
    (0x09C7, 0x09C8),
    (0x09CB, 0x09CD),
    (0x09D7, 0x09D7),
    (0x09DC, 0x09DD),
    (0x09DF, 0x09E3),
    (0x09E6, 0x09F1),
    (0x0A02, 0x0A02),
    (0x0A05, 0x0A0A),
    (0x0A0F, 0x0A10),
    (0x0A13, 0x0A28),
    (0x0A2A, 0x0A30),
    (0x0A32, 0x0A33),
    (0x0A35, 0x0A36),
    (0x0A38, 0x0A39),
    (0x0A3C, 0x0A3C),
    (0x0A3E, 0x0A42),
    (0x0A47, 0x0A48),
    (0x0A4B, 0x0A4D),
    (0x0A59, 0x0A5C),
    (0x0A5E, 0x0A5E),
    (0x0A66, 0x0A74),
    (0x0A81, 0x0A83),
    (0x0A85, 0x0A8B),
    (0x0A8D, 0x0A8D),
    (0x0A8F, 0x0A91),
    (0x0A93, 0x0AA8),
    (0x0AAA, 0x0AB0),
    (0x0AB2, 0x0AB3),
    (0x0AB5, 0x0AB9),
    (0x0ABC, 0x0AC5),
    (0x0AC7, 0x0AC9),
    (0x0ACB, 0x0ACD),
    (0x0AE0, 0x0AE0),
    (0x0AE6, 0x0AEF),
    (0x0B01, 0x0B03),
    (0x0B05, 0x0B0C),
    (0x0B0F, 0x0B10),
    (0x0B13, 0x0B28),
    (0x0B2A, 0x0B30),
    (0x0B32, 0x0B33),
    (0x0B36, 0x0B39),
    (0x0B3C, 0x0B43),
    (0x0B47, 0x0B48),
    (0x0B4B, 0x0B4D),
    (0x0B56, 0x0B57),
    (0x0B5C, 0x0B5D),
    (0x0B5F, 0x0B61),
    (0x0B66, 0x0B6F),
    (0x0B82, 0x0B83),
    (0x0B85, 0x0B8A),
    (0x0B8E, 0x0B90),
    (0x0B92, 0x0B95),
    (0x0B99, 0x0B9A),
    (0x0B9C, 0x0B9C),
    (0x0B9E, 0x0B9F),
    (0x0BA3, 0x0BA4),
    (0x0BA8, 0x0BAA),
    (0x0BAE, 0x0BB5),
    (0x0BB7, 0x0BB9),
    (0x0BBE, 0x0BC2),
    (0x0BC6, 0x0BC8),
    (0x0BCA, 0x0BCD),
    (0x0BD7, 0x0BD7),
    (0x0BE7, 0x0BEF),
    (0x0C01, 0x0C03),
    (0x0C05, 0x0C0C),
    (0x0C0E, 0x0C10),
    (0x0C12, 0x0C28),
    (0x0C2A, 0x0C33),
    (0x0C35, 0x0C39),
    (0x0C3E, 0x0C44),
    (0x0C46, 0x0C48),
    (0x0C4A, 0x0C4D),
    (0x0C55, 0x0C56),
    (0x0C60, 0x0C61),
    (0x0C66, 0x0C6F),
    (0x0C82, 0x0C83),
    (0x0C85, 0x0C8C),
    (0x0C8E, 0x0C90),
    (0x0C92, 0x0CA8),
    (0x0CAA, 0x0CB3),
    (0x0CB5, 0x0CB9),
    (0x0CBE, 0x0CC4),
    (0x0CC6, 0x0CC8),
    (0x0CCA, 0x0CCD),
    (0x0CD5, 0x0CD6),
    (0x0CDE, 0x0CDE),
    (0x0CE0, 0x0CE1),
    (0x0CE6, 0x0CEF),
    (0x0D02, 0x0D03),
    (0x0D05, 0x0D0C),
    (0x0D0E, 0x0D10),
    (0x0D12, 0x0D28),
    (0x0D2A, 0x0D39),
    (0x0D3E, 0x0D43),
    (0x0D46, 0x0D48),
    (0x0D4A, 0x0D4D),
    (0x0D57, 0x0D57),
    (0x0D60, 0x0D61),
    (0x0D66, 0x0D6F),
    (0x0E01, 0x0E2E),
    (0x0E30, 0x0E3A),
    (0x0E40, 0x0E4E),
    (0x0E50, 0x0E59),
    (0x0E81, 0x0E82),
    (0x0E84, 0x0E84),
    (0x0E87, 0x0E88),
    (0x0E8A, 0x0E8A),
    (0x0E8D, 0x0E8D),
    (0x0E94, 0x0E97),
    (0x0E99, 0x0E9F),
    (0x0EA1, 0x0EA3),
    (0x0EA5, 0x0EA5),
    (0x0EA7, 0x0EA7),
    (0x0EAA, 0x0EAB),
    (0x0EAD, 0x0EAE),
    (0x0EB0, 0x0EB9),
    (0x0EBB, 0x0EBD),
    (0x0EC0, 0x0EC4),
    (0x0EC6, 0x0EC6),
    (0x0EC8, 0x0ECD),
    (0x0ED0, 0x0ED9),
    (0x0F18, 0x0F19),
    (0x0F20, 0x0F29),
    (0x0F35, 0x0F35),
    (0x0F37, 0x0F37),
    (0x0F39, 0x0F39),
    (0x0F3E, 0x0F47),
    (0x0F49, 0x0F69),
    (0x0F71, 0x0F84),
    (0x0F86, 0x0F8B),
    (0x0F90, 0x0F95),
    (0x0F97, 0x0F97),
    (0x0F99, 0x0FAD),
    (0x0FB1, 0x0FB7),
    (0x0FB9, 0x0FB9),
    (0x10A0, 0x10C5),
    (0x10D0, 0x10F6),
    (0x1100, 0x1100),
    (0x1102, 0x1103),
    (0x1105, 0x1107),
    (0x1109, 0x1109),
    (0x110B, 0x110C),
    (0x110E, 0x1112),
    (0x113C, 0x113C),
    (0x113E, 0x113E),
    (0x1140, 0x1140),
    (0x114C, 0x114C),
    (0x114E, 0x114E),
    (0x1150, 0x1150),
    (0x1154, 0x1155),
    (0x1159, 0x1159),
    (0x115F, 0x1161),
    (0x1163, 0x1163),
    (0x1165, 0x1165),
    (0x1167, 0x1167),
    (0x1169, 0x1169),
    (0x116D, 0x116E),
    (0x1172, 0x1173),
    (0x1175, 0x1175),
    (0x119E, 0x119E),
    (0x11A8, 0x11A8),
    (0x11AB, 0x11AB),
    (0x11AE, 0x11AF),
    (0x11B7, 0x11B8),
    (0x11BA, 0x11BA),
    (0x11BC, 0x11C2),
    (0x11EB, 0x11EB),
    (0x11F0, 0x11F0),
    (0x11F9, 0x11F9),
    (0x1E00, 0x1E9B),
    (0x1EA0, 0x1EF9),
    (0x1F00, 0x1F15),
    (0x1F18, 0x1F1D),
    (0x1F20, 0x1F45),
    (0x1F48, 0x1F4D),
    (0x1F50, 0x1F57),
    (0x1F59, 0x1F59),
    (0x1F5B, 0x1F5B),
    (0x1F5D, 0x1F5D),
    (0x1F5F, 0x1F7D),
    (0x1F80, 0x1FB4),
    (0x1FB6, 0x1FBC),
    (0x1FBE, 0x1FBE),
    (0x1FC2, 0x1FC4),
    (0x1FC6, 0x1FCC),
    (0x1FD0, 0x1FD3),
    (0x1FD6, 0x1FDB),
    (0x1FE0, 0x1FEC),
    (0x1FF2, 0x1FF4),
    (0x1FF6, 0x1FFC),
    (0x20D0, 0x20DC),
    (0x20E1, 0x20E1),
    (0x2126, 0x2126),
    (0x212A, 0x212B),
    (0x212E, 0x212E),
    (0x2180, 0x2182),
    (0x3005, 0x3005),
    (0x3007, 0x3007),
    (0x3021, 0x302F),
    (0x3031, 0x3035),
    (0x3041, 0x3094),
    (0x3099, 0x309A),
    (0x309D, 0x309E),
    (0x30A1, 0x30FA),
    (0x30FC, 0x30FE),
    (0x3105, 0x312C),
    (0x4E00, 0x9FA5),
    (0xAC00, 0xD7A3),
];

// The IANA names of `com.sun.org.apache.xerces.internal.util.EncodingMap` of the oracle's JDK
// (dumped with `xmlprobe/src/EncodingDump.java`), by the reader Xerces makes for them.
// Java: com/sun/org/apache/xerces/internal/util/EncodingMap.java EncodingMap

/// Read by `Latin1Reader`.
const LATIN1_NAMES: &[&str] = &[
    "CP819",
    "CSISOLATIN1",
    "IBM-819",
    "IBM819",
    "ISO-8859-1",
    "ISO-IR-100",
    "ISO_8859-1",
    "L1",
    "LATIN1",
];

/// Read by `ASCIIReader`.
const ASCII_NAMES: &[&str] = &[
    "ANSI_X3.4-1968",
    "ANSI_X3.4-1986",
    "ASCII",
    "CP367",
    "CSASCII",
    "IBM-367",
    "IBM367",
    "ISO-IR-6",
    "ISO646-US",
    "ISO_646.IRV:1991",
    "US",
    "US-ASCII",
];

/// Read through a Java `Charset` the oracle's JDK supports (not ported).
const OTHER_SUPPORTED_NAMES: &[&str] = &[
    "437",
    "850",
    "852",
    "855",
    "857",
    "862",
    "866",
    "ARABIC",
    "ASMO-708",
    "BIG5",
    "CCSID00858",
    "CP00858",
    "CP437",
    "CP775",
    "CP850",
    "CP852",
    "CP855",
    "CP857",
    "CP862",
    "CP866",
    "CP936",
    "CSBIG5",
    "CSEUCKR",
    "CSEUCPKDFMTJAPANESE",
    "CSGB2312",
    "CSIBM855",
    "CSIBM857",
    "CSIBM866",
    "CSISO13JISC6220JP",
    "CSISO159JISX02121990",
    "CSISO87JISX0208",
    "CSISOLATIN2",
    "CSISOLATIN3",
    "CSISOLATIN4",
    "CSISOLATIN5",
    "CSISOLATINARABIC",
    "CSISOLATINCYRILLIC",
    "CSISOLATINGREEK",
    "CSISOLATINHEBREW",
    "CSKOI8R",
    "CSKSC56011987",
    "CSPC775BALTIC",
    "CSPC850MULTILINGUAL",
    "CSPC862LATINHEBREW",
    "CSPC8CODEPAGE437",
    "CSPCP852",
    "CSSHIFTJIS",
    "CSWINDOWS31J",
    "CYRILLIC",
    "ECMA-114",
    "ECMA-118",
    "ELOT_928",
    "EUC-JP",
    "EUC-KR",
    "EXTENDED_UNIX_CODE_PACKED_FORMAT_FOR_JAPANESE",
    "GB18030",
    "GB2312",
    "GBK",
    "GREEK",
    "GREEK8",
    "HEBREW",
    "IBM-437",
    "IBM-775",
    "IBM-850",
    "IBM-852",
    "IBM-855",
    "IBM-857",
    "IBM-858",
    "IBM-862",
    "IBM-866",
    "IBM00858",
    "IBM437",
    "IBM775",
    "IBM850",
    "IBM852",
    "IBM855",
    "IBM857",
    "IBM862",
    "IBM866",
    "ISO-8859-13",
    "ISO-8859-15",
    "ISO-8859-2",
    "ISO-8859-3",
    "ISO-8859-4",
    "ISO-8859-5",
    "ISO-8859-6",
    "ISO-8859-7",
    "ISO-8859-8",
    "ISO-8859-8-I",
    "ISO-8859-9",
    "ISO-IR-101",
    "ISO-IR-109",
    "ISO-IR-110",
    "ISO-IR-126",
    "ISO-IR-127",
    "ISO-IR-138",
    "ISO-IR-144",
    "ISO-IR-148",
    "ISO-IR-149",
    "ISO-IR-159",
    "ISO-IR-87",
    "ISO_8859-15",
    "ISO_8859-2",
    "ISO_8859-3",
    "ISO_8859-4",
    "ISO_8859-5",
    "ISO_8859-6",
    "ISO_8859-7",
    "ISO_8859-8",
    "ISO_8859-9",
    "KOI8-R",
    "KOREAN",
    "KSC_5601",
    "KS_C_5601-1987",
    "KS_C_5601-1989",
    "L2",
    "L3",
    "L4",
    "L5",
    "LATIN-9",
    "LATIN2",
    "LATIN3",
    "LATIN4",
    "LATIN5",
    "MS936",
    "MS_KANJI",
    "SHIFT_JIS",
    "TIS-620",
    "WINDOWS-1250",
    "WINDOWS-1251",
    "WINDOWS-1252",
    "WINDOWS-1253",
    "WINDOWS-1254",
    "WINDOWS-1257",
    "WINDOWS-31J",
    "WINDOWS-936",
    "X0201",
    "X0208",
    "X0212",
];
