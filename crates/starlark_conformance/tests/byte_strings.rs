//! Spike for docs/research/07-byte-strings.md, option A ("internal" Latin-1 strings).
//!
//! Bazel 9.3.2 reads BUILD/.bzl files and `repository_ctx.read` results as Latin-1, so every
//! Starlark string element is one byte of the UTF-8 file (`--internal_starlark_utf_8_byte_strings`
//! defaults to true). Option A does the same in Blaze: decode source and file bytes as Latin-1
//! into a Rust `String` (one `char` <= U+00FF per byte) and encode back to bytes at the I/O
//! boundaries. starlark-rust then sees code points that *are* bytes.
//!
//! The cases are the lines of `scripts/bytes/workspace/cases.bzl`; expected values come from the
//! oracle (`scripts/bytes/run`, Bazel 9.3.2, 2026-11-18; see `ORACLE`). Each case is evaluated in
//! its own module twice: `Mode::Utf8` (Blaze today) and `Mode::Latin1` (option A, no starlark-rust
//! patch). `LATIN1_GAPS` lists the cases option A still gets wrong; each needs a patch named in
//! the research note. The test pins both lists, so a patch that fixes a gap fails it until the
//! list is updated.
//!
//! aspect_tools_telemetry's `sha1.bzl` and bazel_lib's `lib/private/strings.bzl` are third-party
//! files; the test takes them from the workspace that `scripts/bytes/run` writes
//! (`~/.cache/blaze/work/bytes/third_party/telemetry/`) and skips (with a message) without it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use blaze_starlark::FileKind;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::env::NativeRegistry;
use blaze_starlark::eval;
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::CollectingEvents;
use starlark::environment::FrozenModule;
use starlark::environment::GlobalsBuilder;
use starlark::starlark_module;
use starlark::values::none::NoneType;

/// Oracle values (Bazel 9.3.2 via `scripts/bytes/run`), `str(value)` per case.
const ORACLE: &[(&str, &str)] = &[
    ("len_e", "2"),
    ("len_pig", "4"),
    ("len_mix", "8"),
    ("bytes_e", "[195, 169]"),
    ("bytes_pig", "[240, 159, 144, 183]"),
    ("index_e0", "[195]"),
    ("index_pig_neg1", "[183]"),
    ("slice_pig_1_3", "[159, 144]"),
    ("slice_step2", "[97, 169, 159, 183]"),
    ("elems_len", "6"),
    ("elems_e", "[[195], [169]]"),
    ("find_b", "7"),
    ("index_pig", "3"),
    ("rfind_e", "3"),
    ("count_cont", "2"),
    ("octal_eq", "True"),
    ("octal_len", "1"),
    ("join_bytes_eq", "True"),
    ("in_cont", "True"),
    ("split_cont", "[[97, 226], [172, 98]]"),
    ("replace_cont", "[195, 120]"),
    ("startswith_lead", "True"),
    ("partition_cont", "[[195], [169], []]"),
    ("removeprefix_lead", "[169]"),
    ("rejoin_halves", "True"),
    ("upper_e", "[65, 195, 169]"),
    ("lower_E", "[97, 195, 137]"),
    ("upper_zh", "[228, 184, 173]"),
    ("capitalize_zh", "[196, 184, 173]"),
    ("capitalize_e", "[195, 169, 97]"),
    ("title_ete", "[195, 169, 84, 227, 169]"),
    ("title_x_zh_y", "[88, 228, 184, 173, 89]"),
    ("istitle_E", "False"),
    ("isalpha_e", "False"),
    ("isalnum_e", "False"),
    ("isdigit_e", "False"),
    ("islower_ae", "True"),
    ("isupper_AE", "True"),
    ("isspace_nel", "True"),
    ("isspace_nbsp", "True"),
    ("strip_spaces_e", "[195, 169]"),
    ("strip_A_ring", "[195, 133]"),
    ("strip_nbsp", "[160, 97, 160]"),
    ("lstrip_lead", "[169]"),
    ("splitlines_nel", "[[97, 194, 133, 98], [99]]"),
    (
        "repr_e_ctl",
        "[34, 195, 169, 92, 120, 48, 49, 127, 128, 34]",
    ),
    ("str_list", "[91, 34, 195, 169, 34, 93]"),
    ("pct_s_r", "[195, 169, 124, 34, 195, 169, 34]"),
    ("format_e", "[195, 169, 124, 50]"),
    ("json_encode_e", "[34, 195, 169, 34]"),
    ("json_encode_ff", "[34, 255, 34]"),
    (
        "json_encode_dict",
        "[123, 34, 195, 169, 34, 58, 34, 240, 159, 144, 183, 34, 125]",
    ),
    (
        "json_encode_ctl",
        "[34, 92, 117, 48, 48, 48, 49, 127, 128, 34]",
    ),
    ("json_decode_u00e9", "[195, 169]"),
    ("json_decode_pair", "[240, 159, 144, 183]"),
    ("json_decode_lone", "[239, 191, 189]"),
    ("json_decode_raw", "[195, 169]"),
    ("json_decode_len", "2"),
    ("hash_e", "6214"),
    ("hash_pig", "7307286"),
    (
        "sorted",
        "[[97], [122], [195, 169], [240, 159, 144, 183], [255]]",
    ),
    ("lt_e_z", "False"),
    ("reversed_e", "[169, 195]"),
    ("repeat_len", "6"),
    ("dict_octal_key", "1"),
    ("sha1_e_nl", "6ee66ed9126aa6d0e594acd7c5a70bf6d0b06b78"),
    ("sha1_pig", "b08429ef5e1ac12c6f56e95864cf07ca6366f8f3"),
    ("sha1_ascii", "8ab147eb765ddc533c45b215ce78f84f11d9d510"),
    ("sha1_readme", "af20f6999927fe003d3ee76f76dc8ae5918a0a86"),
];

/// Cases option A (Latin-1 decoding alone, stock vendored starlark-rust and Blaze json) still
/// gets wrong, with the patch that fixes each (07-byte-strings.md section 4).
const LATIN1_GAPS: &[(&str, &str)] = &[
    (
        "islower_ae",
        "islower()/isupper(): ASCII [a-z]/[A-Z] matchers (StringModule.java:873)",
    ),
    (
        "json_decode_len",
        "json.decode: \\u escapes appended as UTF-8 bytes (Json.java:563)",
    ),
    (
        "json_decode_lone",
        "json.decode: \\u escapes appended as UTF-8 bytes (Json.java:563)",
    ),
    (
        "json_decode_pair",
        "json.decode: \\u escapes appended as UTF-8 bytes (Json.java:563)",
    ),
    (
        "json_decode_u00e9",
        "json.decode: \\u escapes appended as UTF-8 bytes (Json.java:563)",
    ),
    (
        "lower_E",
        "lower(): ASCII only (Ascii.toLowerCase, StringModule.java:175)",
    ),
    (
        "pct_s_r",
        "repr: escape only chars < 0x20 (Printer.java:239 escapeCharacter)",
    ),
    (
        "repr_e_ctl",
        "repr: escape only chars < 0x20 (Printer.java:239 escapeCharacter)",
    ),
    (
        "str_list",
        "repr: escape only chars < 0x20 (Printer.java:239 escapeCharacter)",
    ),
    (
        "strip_A_ring",
        "strip(): ASCII whitespace under byte strings (StringModule.java:206)",
    ),
    (
        "strip_nbsp",
        "strip(): ASCII whitespace under byte strings (StringModule.java:206)",
    ),
    (
        "upper_zh",
        "upper(): ASCII only (Ascii.toUpperCase, StringModule.java:183)",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Mode {
    /// Blaze today: sources and file contents decoded as UTF-8.
    Utf8,
    /// Option A: sources and file contents decoded as Latin-1 (Java `ParserInput.fromLatin1`,
    /// `readContent(path, ISO_8859_1)`).
    Latin1,
}

/// Java `new String(bytes, ISO_8859_1)`: one char per byte.
fn latin1_decode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// Java `StringUnsafe.getInternalStringBytes` / `getBytes(ISO_8859_1)` of an internal string.
fn latin1_encode(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| u8::try_from(c as u32).expect("internal string holds a char above U+00FF"))
        .collect()
}

fn decode(mode: Mode, bytes: &[u8]) -> String {
    match mode {
        Mode::Utf8 => String::from_utf8(bytes.to_vec()).expect("UTF-8 source"),
        Mode::Latin1 => latin1_decode(bytes),
    }
}

fn encode(mode: Mode, s: &str) -> Vec<u8> {
    match mode {
        Mode::Utf8 => s.as_bytes().to_vec(),
        Mode::Latin1 => latin1_encode(s),
    }
}

// The repository_ctx boundary, once per mode (StarlarkBaseExternalContext.readFile/createFile).
#[starlark_module]
fn rctx_utf8(builder: &mut GlobalsBuilder) {
    fn rctx_read(path: &str) -> anyhow::Result<String> {
        Ok(decode(Mode::Utf8, &std::fs::read(path)?))
    }
    fn rctx_file(path: &str, content: &str) -> anyhow::Result<NoneType> {
        std::fs::write(path, encode(Mode::Utf8, content))?;
        Ok(NoneType)
    }
}

#[starlark_module]
fn rctx_latin1(builder: &mut GlobalsBuilder) {
    fn rctx_read(path: &str) -> anyhow::Result<String> {
        // Java: StarlarkBaseExternalContext.java:1643 readContent(p.getPath(), ISO_8859_1)
        Ok(decode(
            Mode::Latin1,
            &std::fs::read(latin1_encode(path).as_slice().to_os())?,
        ))
    }
    fn rctx_file(path: &str, content: &str) -> anyhow::Result<NoneType> {
        // Java: StarlarkBaseExternalContext.java:1516 getInternalStringBytes(content)
        std::fs::write(
            latin1_encode(path).as_slice().to_os(),
            encode(Mode::Latin1, content),
        )?;
        Ok(NoneType)
    }
}

/// Internal path bytes to an OS path (Java: the JVM's sun.jnu.encoding=ISO-8859-1 on Linux).
trait ToOs {
    fn to_os(&self) -> PathBuf;
}

impl ToOs for &[u8] {
    fn to_os(&self) -> PathBuf {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(self))
    }
}

fn workspace() -> Option<PathBuf> {
    let ws = PathBuf::from(std::env::var("HOME").ok()?).join(".cache/blaze/work/bytes");
    ws.join("third_party/telemetry/sha1.bzl")
        .exists()
        .then_some(ws)
}

fn template(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/bytes/workspace")
        .join(name)
}

struct Fixture {
    mode: Mode,
    loads: LoadMap,
}

fn exec(
    mode: Mode,
    name: &str,
    source: &[u8],
    loads: &LoadMap,
) -> Result<(FrozenModule, Vec<String>), String> {
    let semantics = StarlarkSemantics::default();
    let events = Arc::new(CollectingEvents::default());
    let ctx = ThreadContext::new(Phase::BzlLoad, Arc::new(semantics.clone()), events.clone());
    let mut registry = NativeRegistry::with_blaze_starlark_builtins();
    registry.register(
        &[FileKind::Bzl],
        match mode {
            Mode::Utf8 => rctx_utf8,
            Mode::Latin1 => rctx_latin1,
        },
    );
    let globals = registry.build(FileKind::Bzl, &semantics);
    let m = eval::compile(FileKind::Bzl, name, decode(mode, source))
        .and_then(|c| eval::exec_module(c, &globals, loads, &ctx, &Default::default()))
        .map_err(|e| e.message)?;
    let printed = events.take().into_iter().map(|e| e.message).collect();
    Ok((m, printed))
}

impl Fixture {
    fn new(mode: Mode, ws: &std::path::Path) -> Fixture {
        let read = |p: PathBuf| std::fs::read(&p).unwrap_or_else(|e| panic!("{p:?}: {e}"));
        let mut loads = LoadMap::new();
        let strings = exec(
            mode,
            "strings.bzl",
            &read(ws.join("third_party/telemetry/strings.bzl")),
            &loads,
        )
        .unwrap()
        .0;
        loads.insert("//third_party/telemetry:strings.bzl", strings);
        let sha1 = exec(
            mode,
            "sha1.bzl",
            &read(ws.join("third_party/telemetry/sha1.bzl")),
            &loads,
        )
        .unwrap()
        .0;
        loads.insert(":sha1_shim.bzl", sha1.clone());
        loads.insert("//:sha1_shim.bzl", sha1);
        let util = exec(
            mode,
            "bytes_util.bzl",
            &read(template("bytes_util.bzl")),
            &loads,
        )
        .unwrap()
        .0;
        loads.insert(":bytes_util.bzl", util.clone());
        loads.insert("//:bytes_util.bzl", util);
        Fixture { mode, loads }
    }

    /// `str(<expr>)` with cases.bzl's locals, or the error message.
    fn case(&self, expr: &str) -> Result<String, String> {
        let src = format!(
            "load(\":bytes_util.bzl\", \"b\")\nload(\":sha1_shim.bzl\", \"sha1\")\n\
             pig = \"\u{1F437}\"\ne = \"\u{e9}\"\nr = \"%s\" % ({expr},)\n"
        );
        let (m, _) = exec(self.mode, "case.bzl", src.as_bytes(), &self.loads)?;
        Ok(m.get("r").unwrap().value().to_str())
    }
}

/// `(name, expr)` for each case line of cases.bzl.
fn cases() -> Vec<(String, String)> {
    let text = std::fs::read_to_string(template("cases.bzl")).unwrap();
    let re = regex::Regex::new(r#"^\s*\("(\w+)", (.*)\),$"#).unwrap();
    text.lines()
        .filter_map(|l| re.captures(l))
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect()
}

fn run_mode(mode: Mode, ws: &std::path::Path) -> BTreeMap<String, Result<String, String>> {
    let fixture = Fixture::new(mode, ws);
    cases()
        .into_iter()
        .map(|(name, expr)| {
            let r = fixture.case(&expr);
            (name, r)
        })
        .collect()
}

fn mismatches(results: &BTreeMap<String, Result<String, String>>) -> Vec<String> {
    let oracle: BTreeMap<_, _> = ORACLE.iter().copied().collect();
    assert_eq!(results.len(), oracle.len(), "cases.bzl and ORACLE disagree");
    results
        .iter()
        .filter(|(name, got)| got.as_deref() != Ok(oracle[name.as_str()]))
        .map(|(name, _)| name.clone())
        .collect()
}

#[test]
fn byte_string_cases_latin1_vs_utf8() {
    let Some(ws) = workspace() else {
        eprintln!("SKIP: run scripts/bytes/run first (needs ~/.cache/blaze/work/bytes)");
        return;
    };
    eval::run_on_evaluation_thread(move || {
        let utf8 = run_mode(Mode::Utf8, &ws);
        let latin1 = run_mode(Mode::Latin1, &ws);
        let oracle: BTreeMap<_, _> = ORACLE.iter().copied().collect();
        for (name, got) in &latin1 {
            eprintln!(
                "{name:20} oracle={:45} latin1={:45?} utf8={:?}",
                oracle[name.as_str()],
                got,
                utf8[name]
            );
        }
        let bad_latin1 = mismatches(&latin1);
        let bad_utf8 = mismatches(&utf8);
        eprintln!(
            "cases: {}; Blaze today (UTF-8) matches {}; option A (Latin-1) matches {}",
            ORACLE.len(),
            ORACLE.len() - bad_utf8.len(),
            ORACLE.len() - bad_latin1.len()
        );
        let gaps: Vec<String> = LATIN1_GAPS.iter().map(|(n, _)| n.to_string()).collect();
        assert_eq!(
            bad_latin1, gaps,
            "option A mismatches differ from LATIN1_GAPS"
        );
        // The telemetry hash: wrong today, right under option A without any patch.
        for name in ["sha1_e_nl", "sha1_pig", "sha1_readme"] {
            assert!(
                bad_utf8.contains(&name.to_owned()),
                "{name} unexpectedly OK today"
            );
            assert!(!bad_latin1.contains(&name.to_owned()));
        }
    });
}

/// The repository-rule flow of aspect_tools_telemetry's dedup.bzl (`repository_ctx.read` of a
/// README with an emoji, first 4 lines, sha1) plus a byte-splitting slice written back with
/// `repository_ctx.file`. Oracle: RCASE lines and `repo_sliced.txt` from `scripts/bytes/run`.
#[test]
fn repository_ctx_read_hash_and_write() {
    let Some(ws) = workspace() else {
        eprintln!("SKIP: run scripts/bytes/run first (needs ~/.cache/blaze/work/bytes)");
        return;
    };
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("byte_strings");
    std::fs::create_dir_all(&out_dir).unwrap();
    eval::run_on_evaluation_thread(move || {
        let mut got = BTreeMap::new();
        for mode in [Mode::Utf8, Mode::Latin1] {
            let fixture = Fixture::new(mode, &ws);
            let sliced = out_dir.join(format!("sliced-{mode:?}.txt"));
            let prelude = format!(
                "load(\":bytes_util.bzl\", \"b\")\nload(\":sha1_shim.bzl\", \"sha1\")\n\
                 data = rctx_read({src:?})\n\
                 head = \"\\n\".join(data.split(\"\\n\")[:4])\n",
                src = template("readme_head.txt").to_str().unwrap(),
            );
            let src = format!(
                "{prelude}read_len = len(data)\nfirst8 = str(b(data[:8]))\n\
                 rctx_file({sliced:?}, data[:3])\n",
                sliced = sliced.to_str().unwrap(),
            );
            let (m, _) = exec(mode, "repo.bzl", src.as_bytes(), &fixture.loads).unwrap();
            let s = |n: &str| m.get(n).unwrap().value().to_str();
            // sha1 may fail today (None from bazel_lib's ord); keep the error text.
            let hash_src = format!("{prelude}h = sha1(head)\n");
            let hash = match exec(mode, "hash.bzl", hash_src.as_bytes(), &fixture.loads) {
                Ok((m, _)) => m.get("h").unwrap().value().to_str(),
                Err(e) => format!("ERROR {e}"),
            };
            got.insert(
                mode,
                (
                    s("read_len"),
                    s("first8"),
                    hash,
                    std::fs::read(&sliced).unwrap(),
                ),
            );
        }
        let latin1 = &got[&Mode::Latin1];
        let utf8 = &got[&Mode::Utf8];
        eprintln!("utf8:   {utf8:?}\nlatin1: {latin1:?}");
        // Oracle RCASE read_len / read_bytes_first8 / head_sha1, and repo_sliced.txt.
        assert_eq!(latin1.0, "92");
        assert_eq!(latin1.1, "[35, 32, 240, 159, 145, 183, 32, 96]");
        assert_eq!(latin1.2, "b22cdf2a02e97d285f8000588780bbfd7236071f");
        assert_eq!(latin1.3, b"# \xf0");
        assert_ne!(utf8.0, "92");
        assert_ne!(utf8.2, "b22cdf2a02e97d285f8000588780bbfd7236071f");
        assert_ne!(utf8.3, b"# \xf0");
    });
}

/// `ctx.actions.write(content = "é🐷\377\n")` from a BUILD literal. Oracle: the output file holds
/// the raw bytes C3 A9 F0 9F 90 B7 FF 0A (FileWriteAction writes getInternalStringBytes), while
/// `aquery --include_file_write_contents` prints base64 of `contents.getBytes(UTF_8)`, i.e. the
/// UTF-8 of the internal string: "w4PCqcOwwp/CkMK3w78K". Under option A the Rust `String` bytes
/// are exactly that UTF-8, so Java's implicit re-encodings (Fingerprint.addString, protobuf
/// string fields) come out the same without a conversion.
#[test]
fn file_write_bytes_and_aquery_contents() {
    let literal: &[u8] = b"\"\xc3\xa9\xf0\x9f\x90\xb7\\377\\n\"";
    let src = [b"r = ".as_slice(), literal, b"\n"].concat();
    let r = eval::run_on_evaluation_thread(move || {
        let mut out = BTreeMap::new();
        for mode in [Mode::Utf8, Mode::Latin1] {
            let (m, _) = exec(mode, "BUILD.bzl", &src, &LoadMap::new()).unwrap();
            let s = m.get("r").unwrap().value().to_str();
            out.insert(format!("{mode:?}"), s);
        }
        out
    });
    let latin1 = &r["Latin1"];
    assert_eq!(latin1_encode(latin1), b"\xc3\xa9\xf0\x9f\x90\xb7\xff\n");
    assert_eq!(base64(latin1.as_bytes()), "w4PCqcOwwp/CkMK3w78K");
    // Today: "\377" is U+00FF, written as C3 BF; the file differs from the oracle's.
    assert_eq!(r["Utf8"].as_bytes(), b"\xc3\xa9\xf0\x9f\x90\xb7\xc3\xbf\n");
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}
