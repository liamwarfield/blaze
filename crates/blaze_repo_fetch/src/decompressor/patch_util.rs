//! Port of `PatchUtil`, Bazel's native unified-diff patch applier, together with the pieces of
//! `java-diff-utils` 4.12 it calls (`UnifiedDiffUtils.parseUnifiedDiff`, `Patch.applyFuzzy`,
//! `Chunk.verifyChunk`, `ChangeDelta.applyFuzzyToAt`).
//!
//! java-diff-utils indexes Java `ArrayList`s and lets `IndexOutOfBoundsException` escape for
//! some malformed patches; `PatchUtil` catches those and reports their message, so the list
//! operations below return the JDK's exception text.

use std::fmt;

use regex::Regex;
use std::sync::LazyLock;

use blaze_vfs::Path;

use crate::errors::IoException;
use crate::fs_utils;

/// Port of `com.github.difflib.patch.PatchFailedException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFailedException {
    pub message: String,
}

impl PatchFailedException {
    pub fn new(message: impl Into<String>) -> Self {
        PatchFailedException {
            message: message.into(),
        }
    }
}

impl fmt::Display for PatchFailedException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PatchFailedException {}

/// `PatchUtil.apply` throws `IOException | PatchFailedException`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatchError {
    Io(IoException),
    Patch(PatchFailedException),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchError::Io(e) => e.fmt(f),
            PatchError::Patch(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for PatchError {}

impl From<IoException> for PatchError {
    fn from(e: IoException) -> Self {
        PatchError::Io(e)
    }
}

impl From<std::io::Error> for PatchError {
    fn from(e: std::io::Error) -> Self {
        PatchError::Io(e.into())
    }
}

impl From<PatchFailedException> for PatchError {
    fn from(e: PatchFailedException) -> Self {
        PatchError::Patch(e)
    }
}

fn pfe(message: impl Into<String>) -> PatchError {
    PatchError::Patch(PatchFailedException::new(message))
}

// Java \s is [ \t\n\x0B\f\r] and \d is [0-9].
const CHUNK_HEADER_PATTERN: &str = r"^@@[ \t\n\x0B\x0C\r]+-(?:([0-9]+)(?:,([0-9]+))?)[ \t\n\x0B\x0C\r]+\+(?:([0-9]+)(?:,([0-9]+))?)[ \t\n\x0B\x0C\r]+@@$";

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:41 PatchUtil.CHUNK_HEADER_RE
static CHUNK_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(CHUNK_HEADER_PATTERN).unwrap());

/// The possible results of ChunkHeader.check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckResult {
    /// The entire chunk is read
    Complete,
    /// Should continue reading the chunk
    Continue,
    /// The chunk body doesn't match the chunk header's size description
    Error,
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:51 PatchUtil.ChunkHeader
struct ChunkHeader {
    old_size: i32,
    new_size: i32,
}

fn parse_int(s: &str) -> i32 {
    s.parse::<i32>()
        .unwrap_or_else(|_| panic!("NumberFormatException: For input string: \"{}\"", s))
}

impl ChunkHeader {
    fn check(&self, old_line_cnt: i32, new_line_cnt: i32) -> CheckResult {
        if old_line_cnt == self.old_size && new_line_cnt == self.new_size {
            return CheckResult::Complete;
        }
        if old_line_cnt <= self.old_size && new_line_cnt <= self.new_size {
            return CheckResult::Continue;
        }
        CheckResult::Error
    }

    fn new(header: &str) -> Result<ChunkHeader, PatchError> {
        match CHUNK_HEADER_RE.captures(header) {
            Some(m) => Ok(ChunkHeader {
                old_size: m.get(2).map(|s| parse_int(s.as_str())).unwrap_or(1),
                new_size: m.get(4).map(|s| parse_int(s.as_str())).unwrap_or(1),
            }),
            None => Err(pfe(format!("Wrong chunk header: {}", header))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineType {
    OldFile,
    NewFile,
    ChunkHead,
    ChunkAdd,
    ChunkDel,
    ChunkEql,
    GitHeader,
    RenameFrom,
    RenameTo,
    NewMode,
    NewFileMode,
    OtherGitLine,
    ChunkNoNewline,
    Unknown,
}

const GIT_LINE_PREFIXES: &[&str] = &[
    "old mode ",
    "new mode ",
    "deleted file mode ",
    "new file mode ",
    "copy from ",
    "copy to ",
    "rename old ",
    "rename new ",
    "similarity index ",
    "dissimilarity index ",
    "index ",
];

/// `line.indexOf("@@", 2)` as a Java int (-1 if absent).
fn index_of_at_at(line: &str) -> i64 {
    match line.get(2..).and_then(|s| s.find("@@")) {
        Some(i) => (i + 2) as i64,
        None => -1,
    }
}

/// `line.substring(0, pos + 2)`.
fn chunk_header_substring(line: &str) -> &str {
    let pos = index_of_at_at(line);
    &line[..(pos + 2) as usize]
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:111 PatchUtil.getLineType
fn get_line_type(line: &str, is_reading_chunk: bool, is_git_diff: bool) -> LineType {
    // A line starting with '\' is a diff comment line, such as "\ No newline at end of file".
    if line.starts_with('\\') {
        return LineType::ChunkNoNewline;
    }
    if is_reading_chunk {
        if line.starts_with('+') {
            return LineType::ChunkAdd;
        }
        if line.starts_with('-') {
            return LineType::ChunkDel;
        }
        if line.starts_with(' ') || line.is_empty() {
            return LineType::ChunkEql;
        }
    } else {
        if line.starts_with("--- ") {
            return LineType::OldFile;
        }
        if line.starts_with("+++ ") {
            return LineType::NewFile;
        }
        if line.starts_with("diff --git ") {
            return LineType::GitHeader;
        }
        if is_git_diff {
            // Only recognize the following when we saw "diff --git " before.
            if line.starts_with("rename from ") {
                return LineType::RenameFrom;
            }
            if line.starts_with("rename to ") {
                return LineType::RenameTo;
            }
            if line.starts_with("new mode ") {
                return LineType::NewMode;
            }
            if line.starts_with("new file mode ") {
                return LineType::NewFileMode;
            }
            if GIT_LINE_PREFIXES.iter().any(|p| line.starts_with(p)) {
                return LineType::OtherGitLine;
            }
        }
    }
    if line.starts_with("@@")
        && line.rfind("@@") != Some(0)
        && CHUNK_HEADER_RE.is_match(chunk_header_substring(line))
    {
        return LineType::ChunkHead;
    }
    LineType::Unknown
}

/// The lines of `file` decoded as UTF-8 (Unicode strings; malformed input is replaced).
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:166 PatchUtil.readFile
fn read_file(file: &Path) -> Result<Vec<String>, IoException> {
    Ok(fs_utils::read_lines(file)?)
}

/// Writes Unicode lines encoded as UTF-8.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:170 PatchUtil.writeFile
fn write_file(file: &Path, content: &[String]) -> Result<(), IoException> {
    Ok(fs_utils::write_lines_as(file, content)?)
}

fn get_file_permission_value(file: &Path) -> Result<i32, IoException> {
    Ok((if file.is_readable()? { 4 } else { 0 })
        + (if file.is_writable()? { 2 } else { 0 })
        + (if file.is_executable()? { 1 } else { 0 }))
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:195 PatchUtil.setFilePermission
fn set_file_permission(file: &Path, permission: i32) -> Result<(), IoException> {
    file.set_readable((permission & 4) == 4)?;
    file.set_writable((permission & 2) == 2)?;
    file.set_executable((permission & 1) == 1)?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// java-diff-utils 4.12

/// `com.github.difflib.patch.Chunk<String>`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chunk {
    position: i32,
    lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerifyChunk {
    Ok,
    PositionOutOfTarget,
    ContentDoesNotMatchTarget,
}

impl fmt::Display for VerifyChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            VerifyChunk::Ok => "OK",
            VerifyChunk::PositionOutOfTarget => "POSITION_OUT_OF_TARGET",
            VerifyChunk::ContentDoesNotMatchTarget => "CONTENT_DOES_NOT_MATCH_TARGET",
        })
    }
}

/// A Java runtime exception escaping java-diff-utils.
enum DiffFailure {
    Patch(PatchFailedException),
    /// `IndexOutOfBoundsException` with its message.
    IndexOutOfBounds(String),
}

/// `ArrayList.get` / `ArrayList.remove(int)` bounds check (`Objects.checkIndex`).
fn check_index(index: i64, length: usize) -> Result<usize, DiffFailure> {
    if index < 0 || index >= length as i64 {
        return Err(DiffFailure::IndexOutOfBounds(format!(
            "Index {} out of bounds for length {}",
            index, length
        )));
    }
    Ok(index as usize)
}

impl Chunk {
    fn size(&self) -> i32 {
        self.lines.len() as i32
    }

    fn last(&self) -> i32 {
        self.position + self.size() - 1
    }

    // java-diff-utils: Chunk.verifyChunk(List, int, int)
    fn verify_chunk(
        &self,
        target: &[String],
        fuzz: i32,
        position: i32,
    ) -> Result<VerifyChunk, DiffFailure> {
        let start_index = fuzz;
        let last_index = self.size() - fuzz;
        let last = position + self.size() - 1;
        if position + fuzz > target.len() as i32 || last - fuzz > target.len() as i32 {
            return Ok(VerifyChunk::PositionOutOfTarget);
        }
        let mut i = start_index;
        while i < last_index {
            let idx = check_index((position + i) as i64, target.len())?;
            if target[idx] != self.lines[i as usize] {
                return Ok(VerifyChunk::ContentDoesNotMatchTarget);
            }
            i += 1;
        }
        Ok(VerifyChunk::Ok)
    }
}

/// `ChangeDelta<String>`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ChangeDelta {
    source: Chunk,
    target: Chunk,
}

impl ChangeDelta {
    // java-diff-utils: ChangeDelta.applyFuzzyToAt
    fn apply_fuzzy_to_at(
        &self,
        target: &mut Vec<String>,
        fuzz: i32,
        position: i32,
    ) -> Result<(), DiffFailure> {
        let size = self.source.size();
        let mut i = fuzz;
        while i < size - fuzz {
            let idx = check_index((position + fuzz) as i64, target.len())?;
            target.remove(idx);
            i += 1;
        }
        // List.subList(fuzz, target.size() - fuzz)
        let from = fuzz;
        let to = self.target.size() - fuzz;
        if from < 0 {
            return Err(DiffFailure::IndexOutOfBounds(format!(
                "fromIndex = {}",
                from
            )));
        }
        if to > self.target.size() {
            return Err(DiffFailure::IndexOutOfBounds(format!("toIndex = {}", to)));
        }
        if from > to {
            // Not caught by PatchUtil: Bazel crashes with this.
            panic!(
                "IllegalArgumentException: fromIndex({}) > toIndex({})",
                from, to
            );
        }
        for (i, line) in (fuzz..).zip(&self.target.lines[from as usize..to as usize]) {
            let index = position + i;
            if index < 0 || index as usize > target.len() {
                return Err(DiffFailure::IndexOutOfBounds(format!(
                    "Index: {}, Size: {}",
                    index,
                    target.len()
                )));
            }
            target.insert(index as usize, line.clone());
        }
        Ok(())
    }
}

/// `Patch.PatchApplyingContext`.
struct PatchApplyingContext {
    result: Vec<String>,
    max_fuzz: i32,
    last_patch_end: i32,
    current_fuzz: i32,
    default_position: i32,
    before_out_range: bool,
    after_out_range: bool,
}

/// `new Patch<>()` holding `deltas`, then `applyFuzzy(target, maxFuzz)`.
// java-diff-utils: Patch.applyFuzzy
fn apply_fuzzy(
    deltas: &[ChangeDelta],
    target: &[String],
    max_fuzz: i32,
) -> Result<Vec<String>, DiffFailure> {
    let mut ctx = PatchApplyingContext {
        result: target.to_vec(),
        max_fuzz,
        last_patch_end: -1,
        current_fuzz: 0,
        default_position: 0,
        before_out_range: false,
        after_out_range: false,
    };
    // the difference between patch's position and actually applied position
    let mut last_patch_delta = 0;
    for delta in sorted_deltas(deltas) {
        ctx.default_position = delta.source.position + last_patch_delta;
        let patch_position = find_position_fuzzy(&mut ctx, delta)?;
        if 0 <= patch_position {
            let fuzz = ctx.current_fuzz;
            delta.apply_fuzzy_to_at(&mut ctx.result, fuzz, patch_position)?;
            last_patch_delta = patch_position - delta.source.position;
            ctx.last_patch_end = delta.source.last() + last_patch_delta;
        } else {
            // CONFLICT_PRODUCES_EXCEPTION
            return Err(DiffFailure::Patch(PatchFailedException::new(format!(
                "could not apply patch due to {}",
                VerifyChunk::ContentDoesNotMatchTarget
            ))));
        }
    }
    Ok(ctx.result)
}

/// `Patch.getDeltas()`: sorted (stably) by source position.
fn sorted_deltas(deltas: &[ChangeDelta]) -> Vec<&ChangeDelta> {
    let mut v: Vec<&ChangeDelta> = deltas.iter().collect();
    v.sort_by_key(|d| d.source.position);
    v
}

fn find_position_fuzzy(
    ctx: &mut PatchApplyingContext,
    delta: &ChangeDelta,
) -> Result<i32, DiffFailure> {
    for fuzz in 0..=ctx.max_fuzz {
        ctx.current_fuzz = fuzz;
        let found_position = find_position_with_fuzz(ctx, delta, fuzz)?;
        if found_position >= 0 {
            return Ok(found_position);
        }
    }
    Ok(-1)
}

fn find_position_with_fuzz(
    ctx: &mut PatchApplyingContext,
    delta: &ChangeDelta,
    fuzz: i32,
) -> Result<i32, DiffFailure> {
    if delta
        .source
        .verify_chunk(&ctx.result, fuzz, ctx.default_position)?
        == VerifyChunk::Ok
    {
        return Ok(ctx.default_position);
    }
    ctx.before_out_range = false;
    ctx.after_out_range = false;
    let mut more_delta: i32 = 0;
    while more_delta >= 0 {
        let pos = find_position_with_fuzz_and_more_delta(ctx, delta, fuzz, more_delta)?;
        if pos >= 0 {
            return Ok(pos);
        }
        if ctx.before_out_range && ctx.after_out_range {
            break;
        }
        more_delta = more_delta.wrapping_add(1);
    }
    Ok(-1)
}

fn find_position_with_fuzz_and_more_delta(
    ctx: &mut PatchApplyingContext,
    delta: &ChangeDelta,
    fuzz: i32,
    more_delta: i32,
) -> Result<i32, DiffFailure> {
    // range check: can't apply before end of last patch
    if !ctx.before_out_range {
        let begin_at = ctx.default_position - more_delta + fuzz;
        // We can't apply patch before end of last patch.
        if begin_at <= ctx.last_patch_end {
            ctx.before_out_range = true;
        }
    }
    // range check: can't apply after end of result
    if !ctx.after_out_range {
        let begin_at = ctx.default_position + more_delta + delta.source.size() - fuzz;
        if (ctx.result.len() as i32) < begin_at {
            ctx.after_out_range = true;
        }
    }
    if !ctx.before_out_range {
        let before =
            delta
                .source
                .verify_chunk(&ctx.result, fuzz, ctx.default_position - more_delta)?;
        if before == VerifyChunk::Ok {
            return Ok(ctx.default_position - more_delta);
        }
    }
    if !ctx.after_out_range {
        let after =
            delta
                .source
                .verify_chunk(&ctx.result, fuzz, ctx.default_position + more_delta)?;
        if after == VerifyChunk::Ok {
            return Ok(ctx.default_position + more_delta);
        }
    }
    Ok(-1)
}

/// `UnifiedDiffUtils.parseUnifiedDiff`: one `ChangeDelta` per chunk.
// java-diff-utils: UnifiedDiffUtils.parseUnifiedDiff
fn parse_unified_diff(diff: &[String]) -> Vec<ChangeDelta> {
    let mut in_prelude = true;
    let mut raw_chunk: Vec<(char, String)> = Vec::new();
    let mut deltas = Vec::new();
    let mut old_ln = 0;
    let mut new_ln = 0;
    for line in diff {
        // Skip leading lines until after we've seen one starting with '+++'
        if in_prelude {
            if line.starts_with("+++") {
                in_prelude = false;
            }
            continue;
        }
        if let Some(m) = CHUNK_HEADER_RE.captures(line) {
            // Process the lines in the previous chunk
            process_lines_in_prev_chunk(&mut raw_chunk, &mut deltas, old_ln, new_ln);
            // Parse the @@ header
            old_ln = m.get(1).map(|s| parse_int(s.as_str())).unwrap_or(1);
            new_ln = m.get(3).map(|s| parse_int(s.as_str())).unwrap_or(1);
            if old_ln == 0 {
                old_ln = 1;
            }
            if new_ln == 0 {
                new_ln = 1;
            }
        } else if let Some(tag) = line.chars().next() {
            if tag == ' ' || tag == '+' || tag == '-' {
                raw_chunk.push((tag, line[1..].to_string()));
            }
        } else {
            raw_chunk.push((' ', String::new()));
        }
    }
    // Process the lines in the last chunk
    process_lines_in_prev_chunk(&mut raw_chunk, &mut deltas, old_ln, new_ln);
    deltas
}

fn process_lines_in_prev_chunk(
    raw_chunk: &mut Vec<(char, String)>,
    deltas: &mut Vec<ChangeDelta>,
    old_ln: i32,
    new_ln: i32,
) {
    if raw_chunk.is_empty() {
        return;
    }
    let mut old_chunk_lines = Vec::new();
    let mut new_chunk_lines = Vec::new();
    for (tag, rest) in raw_chunk.iter() {
        if *tag == ' ' || *tag == '-' {
            old_chunk_lines.push(rest.clone());
        }
        if *tag == ' ' || *tag == '+' {
            new_chunk_lines.push(rest.clone());
        }
    }
    deltas.push(ChangeDelta {
        source: Chunk {
            position: old_ln - 1,
            lines: old_chunk_lines,
        },
        target: Chunk {
            position: new_ln - 1,
            lines: new_chunk_lines,
        },
    });
    raw_chunk.clear();
}

// ---------------------------------------------------------------------------------------------

fn path_or_null(p: Option<&Path>) -> String {
    p.map(|p| p.to_string())
        .unwrap_or_else(|| "null".to_string())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:207 PatchUtil.applyPatchToFile
fn apply_patch_to_file(
    deltas: &[ChangeDelta],
    old_file: Option<&Path>,
    new_file: Option<&Path>,
    is_renaming: bool,
    mut file_permission: i32,
) -> Result<(), PatchError> {
    // The file we should read oldContent from.
    let input_file = if let Some(f) = old_file.filter(|f| f.exists()) {
        Some(f)
    } else {
        new_file.filter(|f| f.exists())
    };

    let old_content = match input_file {
        None => Vec::new(),
        Some(f) => {
            let c = read_file(f)?;
            // Preserve old file permission if no explicit permission is set.
            if file_permission == -1 {
                file_permission = get_file_permission_value(f)?;
            }
            c
        }
    };

    let mut new_content = old_content;
    // Apply the chunks separately and in reverse order to workaround an issue in applyFuzzy.
    let sorted = sorted_deltas(deltas);
    for delta in sorted.iter().rev() {
        match apply_fuzzy(std::slice::from_ref(*delta), &new_content, 2) {
            Ok(c) => new_content = c,
            Err(e) => {
                let message = match e {
                    DiffFailure::Patch(p) => p.message,
                    DiffFailure::IndexOutOfBounds(m) => m,
                };
                return Err(pfe(format!(
                    "in patch applied to {}: {}, error applying change near line {}",
                    path_or_null(old_file),
                    message,
                    delta.source.position + 1
                )));
            }
        }
    }

    // The file we should write newContent to.
    let output_file = if old_file.is_some_and(|f| f.exists()) && !is_renaming {
        old_file
    } else {
        new_file
    };

    // The old file should always change, therefore we can just delete the original file.
    // If the output file name is the same as the old file, we'll just recreate it later.
    if let Some(f) = old_file {
        f.delete()?;
    }

    // Does this patch look like deleting a file.
    let is_delete_file = new_file.is_none() && new_content.is_empty();

    if let Some(out) = output_file
        && !is_delete_file
    {
        write_file(out, &new_content)?;
        if file_permission != -1 {
            set_file_permission(out, file_permission)?;
        }
    }
    Ok(())
}

/// Strip a number of leading components from a path.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:276 PatchUtil.stripPath
fn strip_path(path: &str, mut strip: i32) -> String {
    let mut pos = 0;
    let bytes = path.as_bytes();
    while pos < bytes.len() && strip > 0 {
        if bytes[pos] == b'/' {
            strip -= 1;
        }
        pos += 1;
    }
    path[pos..].to_string()
}

/// Java `String.trim()`: strips characters `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Extracts the file path from a `--- ` or `+++ ` line; `None` for `/dev/null`.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:293 PatchUtil.extractPath
fn extract_path(line: &str, strip: i32, loc: usize) -> Result<Option<String>, PatchError> {
    assert!(line.starts_with("+++ ") || line.starts_with("--- "));
    let line = line.split('\t').next().unwrap();
    if line.len() > 4 {
        let path = java_trim(&line[4..]);
        if path == "/dev/null" {
            return Ok(None);
        }
        let path = strip_path(path, strip);
        if !path.is_empty() {
            return Ok(Some(path));
        }
    }
    Err(pfe(format!(
        "Cannot determine file name with strip = {} at line {}:\n{}",
        strip, loc, line
    )))
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:317 PatchUtil.getFilePath
fn get_file_path(
    path: Option<&str>,
    output_directory: &Path,
    loc: usize,
) -> Result<Option<Path>, PatchError> {
    let Some(path) = path else {
        return Ok(None);
    };
    // `path` is Unicode (the patch is read as UTF-8); Java puts it into the path unconverted.
    let file_path =
        output_directory.get_relative_str(&fs_utils::unicode_path_as_seen_by_file_system(path));
    if !file_path.starts_with(output_directory) {
        return Err(pfe(format!(
            "Cannot patch file outside of external repository ({}), file path = \"{}\" at line {}",
            output_directory.get_path_string(),
            path,
            loc
        )));
    }
    Ok(Some(file_path))
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:333 PatchUtil.checkPatchContentIsComplete
fn check_patch_content_is_complete(
    patch_content: &[String],
    header: Option<&ChunkHeader>,
    old_line_count: i32,
    new_line_count: i32,
    loc: usize,
) -> Result<(), PatchError> {
    // If the patchContent is not empty, it should have correct format.
    if !patch_content.is_empty() {
        if patch_content.len() < 2
            || !patch_content[0].starts_with("---")
            || !patch_content[1].starts_with("+++")
        {
            return Err(pfe(format!(
                "The patch content must start with ---/+++ prelude lines at line {}.",
                loc
            )));
        }
        let Some(header) = header else {
            return Err(pfe(format!(
                "Looks like a unified diff at line {}, but no patch chunk was found.",
                loc
            )));
        };
        // result will never be Result.Error here because it would have been throw in previous
        // line already.
        if header.check(old_line_count, new_line_count) == CheckResult::Continue {
            return Err(pfe(format!(
                "Expecting more chunk line at line {}",
                loc + patch_content.len()
            )));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:362 PatchUtil.checkFilesStatusForRenaming
fn check_files_status_for_renaming(
    old_file: Option<&Path>,
    new_file: Option<&Path>,
    old_file_str: Option<&str>,
    new_file_str: Option<&str>,
    loc: usize,
) -> Result<(), PatchError> {
    // If we're doing a renaming, old file should be specified and exists, new file should be
    // specified but doesn't exist yet.
    let mut old_file_error = String::new();
    let mut new_file_error = String::new();
    match old_file {
        None => old_file_error = ", old file name is not specified".to_string(),
        Some(f) if !f.exists() => {
            old_file_error = format!(
                ", old file name ({}) doesn't exist",
                old_file_str.unwrap_or("null")
            )
        }
        _ => {}
    }
    match new_file {
        None => new_file_error = ", new file name is not specified".to_string(),
        Some(f) if f.exists() => {
            new_file_error = format!(
                ", new file name ({}) already exists",
                new_file_str.unwrap_or("null")
            )
        }
        _ => {}
    }
    if !old_file_error.is_empty() || !new_file_error.is_empty() {
        return Err(pfe(format!(
            "Cannot rename file (near line {}){}{}.",
            loc, old_file_error, new_file_error
        )));
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:386 PatchUtil.checkFilesStatusForPatching
fn check_files_status_for_patching(
    deltas: &[ChangeDelta],
    old_file: Option<&Path>,
    new_file: Option<&Path>,
    old_file_str: Option<&str>,
    new_file_str: Option<&str>,
    loc: usize,
) -> Result<(), PatchError> {
    // At least one of oldFile or newFile should be specified.
    if old_file.is_none() && new_file.is_none() {
        return Err(pfe(format!(
            "Wrong patch format near line {}, neither new file or old file are specified.",
            loc
        )));
    }
    // Does this patch look like adding a new file.
    let is_add_file = deltas.len() == 1 && sorted_deltas(deltas)[0].source.lines.is_empty();
    // If this patch is not adding a new file, then either old file or new file should be
    // specified and exists, if not we throw an error.
    if !is_add_file
        && !old_file.is_some_and(|f| f.exists())
        && !new_file.is_some_and(|f| f.exists())
    {
        let old_file_error = match old_file {
            None => ", old file name is not specified".to_string(),
            Some(_) => format!(
                ", old file name ({}) doesn't exist",
                old_file_str.unwrap_or("null")
            ),
        };
        let new_file_error = match new_file {
            None => ", new file name is not specified".to_string(),
            Some(_) => format!(
                ", new file name ({}) doesn't exist",
                new_file_str.unwrap_or("null")
            ),
        };
        return Err(pfe(format!(
            "Cannot find file to patch (near line {}){}{}.",
            loc, old_file_error, new_file_error
        )));
    }
    Ok(())
}

/// Apply a patch file under a directory.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:432 PatchUtil.apply
pub fn apply(patch_file: &Path, strip: i32, output_directory: &Path) -> Result<(), PatchError> {
    apply_internal(patch_file, strip, output_directory, None)
}

/// Apply a patch file under a directory, skipping all parts of the patch file that do not apply
/// to the given single file. Renaming the file is not supported in this case.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:448 PatchUtil.applyToSingleFile
pub fn apply_to_single_file(
    patch_file: &Path,
    strip: i32,
    output_directory: &Path,
    single_file: &Path,
) -> Result<(), PatchError> {
    apply_internal(patch_file, strip, output_directory, Some(single_file))
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/PatchUtil.java:454 PatchUtil.applyInternal
fn apply_internal(
    patch_file: &Path,
    strip: i32,
    output_directory: &Path,
    single_file: Option<&Path>,
) -> Result<(), PatchError> {
    if !patch_file.exists() {
        return Err(pfe(format!(
            "Cannot find patch file: {}",
            patch_file.get_path_string()
        )));
    }

    let single_file_str: Option<String> = single_file.map(|f| {
        f.relative_to(output_directory)
            .get_path_string()
            .to_string()
    });
    let mut is_git_diff = false;
    let mut has_rename_from = false;
    let mut has_rename_to = false;
    let mut is_reading_chunk = false;
    let mut patch_content: Vec<String> = Vec::new();
    let mut header: Option<ChunkHeader> = None;
    let mut old_file_str: Option<String> = None;
    let mut new_file_str: Option<String> = None;
    let mut old_file: Option<Path> = None;
    let mut new_file: Option<Path> = None;
    let mut old_line_count = 0;
    let mut new_line_count = 0;
    let mut file_permission = -1;

    let patch_file_lines = read_file(patch_file)?;
    let dollar = "$".to_string();
    for i in 0..=patch_file_lines.len() {
        // Adding an extra line to make sure last chunk also gets applied.
        let line: &String = if i < patch_file_lines.len() {
            &patch_file_lines[i]
        } else {
            &dollar
        };
        let type_ = get_line_type(line, is_reading_chunk, is_git_diff);
        match type_ {
            LineType::OldFile => {
                patch_content.push(line.clone());
                old_file_str = extract_path(line, strip, i + 1)?;
                old_file = get_file_path(old_file_str.as_deref(), output_directory, i + 1)?;
            }
            LineType::NewFile => {
                patch_content.push(line.clone());
                new_file_str = extract_path(line, strip, i + 1)?;
                new_file = get_file_path(new_file_str.as_deref(), output_directory, i + 1)?;
            }
            LineType::NewMode | LineType::NewFileMode => {
                // The line should look like: "new mode 100755" or "new file mode 100755"
                // 7 is the file permission for owner, which is at index 12 or 17
                let index = if type_ == LineType::NewMode { 12 } else { 17 };
                // Java `line.charAt(index)`: a UTF-16 unit of the Unicode line.
                let c = line.encode_utf16().nth(index).unwrap_or_else(|| {
                    panic!(
                        "StringIndexOutOfBoundsException: index {}, length {}",
                        index,
                        line.encode_utf16().count()
                    )
                });
                if !(u16::from(b'0')..=u16::from(b'7')).contains(&c) {
                    return Err(pfe(format!(
                        "Wrong file mode format at line {}: {}",
                        i + 1,
                        line
                    )));
                }
                file_permission = (c - u16::from(b'0')) as i32;
            }
            LineType::ChunkHead => {
                let header_str = chunk_header_substring(line).to_string();
                patch_content.push(header_str.clone());
                header = Some(ChunkHeader::new(&header_str)?);
                old_line_count = 0;
                new_line_count = 0;
                is_reading_chunk = true;
            }
            LineType::ChunkAdd | LineType::ChunkDel | LineType::ChunkEql => {
                let (what, d_old, d_new) = match type_ {
                    LineType::ChunkAdd => ("an added", 0, 1),
                    LineType::ChunkDel => ("a deleted", 1, 0),
                    _ => ("a context", 1, 1),
                };
                old_line_count += d_old;
                new_line_count += d_new;
                patch_content.push(line.clone());
                let h = header
                    .as_ref()
                    .expect("NullPointerException: chunk line without header");
                match h.check(old_line_count, new_line_count) {
                    CheckResult::Complete => is_reading_chunk = false,
                    CheckResult::Error => {
                        return Err(pfe(format!(
                            "Wrong chunk detected near line {}: {}, does not expect {} line here.",
                            i + 1,
                            line,
                            what
                        )));
                    }
                    CheckResult::Continue => {}
                }
            }
            LineType::ChunkNoNewline => {
                if !patch_content.is_empty() && header.is_some() {
                    patch_content.push(line.clone());
                }
            }
            LineType::RenameFrom => {
                has_rename_from = true;
                if old_file_str.is_none() {
                    // len("rename from ") == 12
                    let s = java_trim(&line[12..]).to_string();
                    if s.is_empty() {
                        return Err(pfe(format!(
                            "Cannot determine file name from line {}:\n{}",
                            i + 1,
                            line
                        )));
                    }
                    old_file = get_file_path(Some(&s), output_directory, i + 1)?;
                    old_file_str = Some(s);
                }
            }
            LineType::RenameTo => {
                has_rename_to = true;
                if new_file_str.is_none() {
                    // len("rename to ") == 10
                    let s = java_trim(&line[10..]).to_string();
                    if s.is_empty() {
                        return Err(pfe(format!(
                            "Cannot determine file name from line {}:\n{}",
                            i + 1,
                            line
                        )));
                    }
                    new_file = get_file_path(Some(&s), output_directory, i + 1)?;
                    new_file_str = Some(s);
                }
            }
            LineType::OtherGitLine => {}
            LineType::GitHeader | LineType::Unknown => {
                // A git header line or an unknown line should trigger an action to apply
                // collected patch content to a file.
                // Renaming is a git only format
                let is_renaming = is_git_diff && has_rename_from && has_rename_to;

                if !patch_content.is_empty() || is_renaming || file_permission != -1 {
                    // We collected something useful, let's do some checks before applying.
                    let patch_start_location = i + 1 - patch_content.len();

                    check_patch_content_is_complete(
                        &patch_content,
                        header.as_ref(),
                        old_line_count,
                        new_line_count,
                        patch_start_location,
                    )?;

                    if is_renaming {
                        if let Some(sf) = single_file {
                            if new_file.as_ref() == Some(sf) || old_file.as_ref() == Some(sf) {
                                return Err(pfe(format!(
                                    "Renaming {} while applying patches to it as a single file is not supported.",
                                    sf
                                )));
                            }
                        } else {
                            check_files_status_for_renaming(
                                old_file.as_ref(),
                                new_file.as_ref(),
                                old_file_str.as_deref(),
                                new_file_str.as_deref(),
                                patch_start_location,
                            )?;
                        }
                    }
                    if let Some(sfs) = &single_file_str
                        && strip == 0
                        && Some(format!("a/{}", sfs)) == old_file_str
                        && Some(format!("b/{}", sfs)) == new_file_str
                    {
                        return Err(pfe(format!(
                            "error at line {}: the patch file contains a/b prefixes, did you forget to set patch_strip = 1?",
                            patch_start_location
                        )));
                    }

                    if single_file.is_none()
                        || (new_file.as_ref() == single_file && old_file.as_ref() == single_file)
                    {
                        let deltas = parse_unified_diff(&patch_content);
                        check_files_status_for_patching(
                            &deltas,
                            old_file.as_ref(),
                            new_file.as_ref(),
                            old_file_str.as_deref(),
                            new_file_str.as_deref(),
                            patch_start_location,
                        )?;
                        apply_patch_to_file(
                            &deltas,
                            old_file.as_ref(),
                            new_file.as_ref(),
                            is_renaming,
                            file_permission,
                        )?;
                    }
                }

                patch_content.clear();
                header = None;
                old_file_str = None;
                new_file_str = None;
                old_file = None;
                new_file = None;
                file_permission = -1;
                old_line_count = 0;
                new_line_count = 0;
                is_reading_chunk = false;
                // If the new patch starts with "diff --git " then it's a git diff.
                is_git_diff = type_ == LineType::GitHeader;
                if is_git_diff {
                    // In case there is no line starting with +++ and --- (file permission
                    // change), try to parse the file names from the line starting with
                    // "diff --git"
                    let args: Vec<&str> = line.split(' ').collect();
                    if args.len() >= 4 {
                        let o = strip_path(args[2], strip);
                        if !o.is_empty() {
                            old_file = get_file_path(Some(&o), output_directory, i + 1)?;
                        }
                        old_file_str = Some(o);
                        let n = strip_path(args[3], strip);
                        if !n.is_empty() {
                            new_file = get_file_path(Some(&n), output_directory, i + 1)?;
                        }
                        new_file_str = Some(n);
                    }
                }
                has_rename_from = false;
                has_rename_to = false;
            }
        }
    }
    Ok(())
}
