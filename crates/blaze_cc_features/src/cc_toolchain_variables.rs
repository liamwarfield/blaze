//! Port of `com.google.devtools.build.lib.rules.cpp.CcToolchainVariables`: configured build
//! variables usable by the toolchain configuration.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use parking_lot::Mutex;

use crate::actions::{
    ArtifactRef, InputMetadataProvider, NOOP, OS, PathMapper, path_fragment_create_for_os,
};
use crate::errors::{EvalException, ExpansionException, JavaList};

/// Shared handle to [`CcToolchainVariables`] (Java passes the object by reference).
pub type CcToolchainVariablesRef = Arc<CcToolchainVariables>;

/// A piece of a single string value.
///
/// A single value can contain a combination of text and variables (for example "-f
/// %{var1}/%{var2}"). We split the string into chunks, where each chunk represents either a text
/// snippet, or a variable that is to be replaced.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:73 CcToolchainVariables.StringChunk
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StringChunk {
    /// A plain text chunk of a string (containing no variables).
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:86 CcToolchainVariables.StringLiteralChunk
    StringLiteralChunk(String),
    /// A chunk of a string value into which a variable should be expanded.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:98 CcToolchainVariables.VariableChunk
    VariableChunk(String),
    /// A chunk of an exec path that can be mapped upon expansion.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:115 CcToolchainVariables.RelativePathChunk
    RelativePathChunk(PathFragment),
}

impl StringChunk {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:117 CcToolchainVariables.RelativePathChunk.<init>
    pub fn relative_path_chunk(exec_path: PathFragment) -> StringChunk {
        blaze_util::check_argument!(
            !exec_path.is_absolute(),
            "execPath is not relative: {}",
            exec_path
        );
        StringChunk::RelativePathChunk(exec_path)
    }

    /// Expands this chunk.
    pub fn expand(
        &self,
        variables: &CcToolchainVariables,
        path_mapper: &dyn PathMapper,
    ) -> Result<String, ExpansionException> {
        match self {
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:89 CcToolchainVariables.StringLiteralChunk.expand
            StringChunk::StringLiteralChunk(text) => Ok(text.clone()),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:101 CcToolchainVariables.VariableChunk.expand
            StringChunk::VariableChunk(variable_name) => {
                // We check all variables in FlagGroup.expandCommandLine.
                // If we arrive here with the variable not being available, the variable was
                // provided, but the nesting level of the NestedSequence was deeper than the
                // nesting level of the flag groups.
                variables.get_string_variable(variable_name, path_mapper)
            }
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:122 CcToolchainVariables.RelativePathChunk.expand
            StringChunk::RelativePathChunk(exec_path) => {
                Ok(path_mapper.map(exec_path).get_path_string().to_string())
            }
        }
    }
}

/// Parser for toolchain string values.
///
/// A string value contains a snippet of text supporting variable expansion. For example, a
/// string value "-f %{var1}/%{var2}" will expand the values of the variables "var1" and "var2" in
/// the corresponding places in the string.
///
/// The `StringValueParser` takes a string and parses it into a list of [`StringChunk`] objects,
/// where each chunk represents either a snippet of text or a variable to be expanded. In the above
/// example, the resulting chunks would be ["-f ", var1, "/", var2].
///
/// To get a literal percent character, "%%" can be used in the string.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:140 CcToolchainVariables.StringValueParser
// PORT: the parser works on byte offsets; a position that is printed is converted to Java's
// index in the internal string (`java_position`).
pub struct StringValueParser {
    value: String,
    /// The current position in `value` during parsing.
    current: usize,
    chunks: Vec<StringChunk>,
    used_variables: indexmap::IndexSet<String>,
}

const PATH_PREFIX: &str = "path:";

impl StringValueParser {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:152 CcToolchainVariables.StringValueParser.<init>
    pub fn new(value: &str) -> Result<StringValueParser, EvalException> {
        let mut parser = StringValueParser {
            value: value.to_string(),
            current: 0,
            chunks: Vec::new(),
            used_variables: indexmap::IndexSet::new(),
        };
        parser.parse()?;
        Ok(parser)
    }

    /// Returns the parsed chunks for this string.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:158 CcToolchainVariables.StringValueParser.getChunks
    pub fn get_chunks(&self) -> Vec<StringChunk> {
        self.chunks.clone()
    }

    /// The variables referenced by the string (Java's unused `usedVariables`).
    pub fn get_used_variables(&self) -> Vec<String> {
        self.used_variables.iter().cloned().collect()
    }

    fn byte_at(&self, i: usize) -> u8 {
        self.value.as_bytes()[i]
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:167 CcToolchainVariables.StringValueParser.parse
    fn parse(&mut self) -> Result<(), EvalException> {
        while self.current < self.value.len() {
            if self.at_variable_start() {
                self.parse_variable_chunk()?;
            } else {
                self.parse_string_chunk();
            }
        }
        Ok(())
    }

    /// Returns whether the current position is the start of a variable.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:180 CcToolchainVariables.StringValueParser.atVariableStart
    fn at_variable_start(&self) -> bool {
        // We parse a variable when value starts with '%', but not '%%'.
        self.byte_at(self.current) == b'%'
            && (self.current + 1 >= self.value.len() || self.byte_at(self.current + 1) != b'%')
    }

    /// Parses a chunk of text until the next '%', which indicates either an escaped literal '%'
    /// or a variable.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:190 CcToolchainVariables.StringValueParser.parseStringChunk
    fn parse_string_chunk(&mut self) {
        let mut start = self.current;
        // We only parse string chunks starting with '%' if they also start with '%%'.
        // In that case, we want to have a single '%' in the string, so we start at the second
        // character.
        // Note that for strings like "abc%%def" this will lead to two string chunks, the first
        // referencing the subtring "abc", and a second referencing the substring "%def".
        if self.byte_at(self.current) == b'%' {
            self.current += 1;
            start = self.current;
        }
        self.current = match index_of(&self.value, b'%', self.current + 1) {
            Some(i) => i,
            None => self.value.len(),
        };
        let text = self.value[start..self.current].to_string();
        self.chunks.push(StringChunk::StringLiteralChunk(text));
    }

    /// Parses a variable to be expanded.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:214 CcToolchainVariables.StringValueParser.parseVariableChunk
    fn parse_variable_chunk(&mut self) -> Result<(), EvalException> {
        self.current += 1;
        if self.current >= self.value.len() || self.byte_at(self.current) != b'{' {
            return Err(self.abort("expected '{'"));
        }
        self.current += 1;
        if self.current >= self.value.len() || self.byte_at(self.current) == b'}' {
            return Err(self.abort("expected variable name"));
        }
        let end = match index_of(&self.value, b'}', self.current) {
            Some(end) => end,
            // PORT: Java's substring(current, -1) throws here.
            None => panic!(
                "StringIndexOutOfBoundsException: begin {}, end -1, length {}",
                self.java_position(self.current),
                self.java_position(self.value.len())
            ),
        };
        let name = self.value[self.current..end].to_string();
        if let Some(path) = name.strip_prefix(PATH_PREFIX) {
            if path.is_empty() {
                return Err(self.abort("expected path after 'path:'"));
            }
            // The provided path is expected to be an exec path, which always uses '/' as a
            // separator and is relative. Ensure that it is parsed consistently.
            let path_fragment = path_fragment_create_for_os(path, OS::LINUX);
            if path_fragment.is_absolute() {
                return Err(self.abort("expected relative Unix-style path after 'path:'"));
            }
            self.chunks
                .push(StringChunk::relative_path_chunk(path_fragment));
        } else {
            self.used_variables.insert(name.clone());
            self.chunks.push(StringChunk::VariableChunk(name));
        }
        self.current = end + 1;
        Ok(())
    }

    /// Java's index (in chars of the internal string) of the byte offset `at`. The parser only
    /// stops at ASCII delimiters or the end, so `at` is a char boundary.
    fn java_position(&self, at: usize) -> usize {
        blaze_util::string_encoding::internal_length(&self.value[..at])
    }

    /// Returns an error with the given text, adding information about the current position in
    /// the string.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:248 CcToolchainVariables.StringValueParser.abort
    fn abort(&self, error: &str) -> EvalException {
        EvalException::new(format!(
            "Invalid toolchain configuration: {} at position {} while parsing a flag containing '{}'",
            error,
            self.java_position(self.current),
            self.value
        ))
    }
}

/// Java's `String.indexOf(char, fromIndex)` on bytes.
fn index_of(s: &str, c: u8, from: usize) -> Option<usize> {
    if from >= s.len() {
        return None;
    }
    s.as_bytes()[from..]
        .iter()
        .position(|&b| b == c)
        .map(|i| i + from)
}

/// Stand-in for Starlark's `net.starlark.java.eval.Structure` as seen by
/// [`StarlarkStructureAdapter`]: the Starlark wrapper crate implements it for Starlark structs
/// (`NamedLibraryInfo`, `ObjectFileGroupInfo`, `VersionedLibraryInfo` from rules_cc).
pub trait Structure: Send + Sync + fmt::Debug {
    /// Java: `Structure.getValue(name)`; `Ok(None)` where Java returns null (no such field).
    fn get_value(&self, name: &str) -> Result<Option<VariableObject>, EvalException>;
    /// Java: `Starlark.type(val)`.
    fn starlark_type(&self) -> String;
    /// Java: `val.truth()`.
    fn truth(&self) -> bool;
}

/// A raw value stored in a variables map or a sequence: Java's `Object`, before
/// `asVariableValue` wraps it.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:826 CcToolchainVariables.asVariableValue
#[derive(Clone, Debug)]
pub enum VariableObject {
    /// Java `null` (not Starlark `None`); reading it is a `NullPointerException` in Java.
    Null,
    /// Starlark `None`: same behavior as an omitted field.
    None,
    Boolean(bool),
    String(Arc<str>),
    /// A `Label`, already rendered with `Label.expandToCommandLine()`.
    // PORT: blaze_cmdline::Label::expand_to_command_line is applied by whoever builds the value.
    Label(Arc<str>),
    Artifact(ArtifactRef),
    PathFragment(PathFragment),
    /// Any Java `Iterable` (Starlark list or tuple, `ImmutableList`).
    Iterable(Arc<Vec<VariableObject>>),
    /// A `NestedSet` or the set of a `Depset`.
    NestedSet(NestedSet<VariableObject>),
    Structure(Arc<dyn Structure>),
    /// An already-wrapped `VariableValue`.
    Value(VariableValue),
}

impl VariableObject {
    pub fn string(s: &str) -> VariableObject {
        VariableObject::String(Arc::from(s))
    }

    /// A list of strings (Java `ImmutableList<String>`).
    pub fn string_list<S: AsRef<str>>(values: impl IntoIterator<Item = S>) -> VariableObject {
        VariableObject::Iterable(Arc::new(
            values
                .into_iter()
                .map(|s| VariableObject::string(s.as_ref()))
                .collect(),
        ))
    }

    pub fn list(values: Vec<VariableObject>) -> VariableObject {
        VariableObject::Iterable(Arc::new(values))
    }
}

impl PartialEq for VariableObject {
    fn eq(&self, other: &Self) -> bool {
        use VariableObject as O;
        match (self, other) {
            (O::Null, O::Null) | (O::None, O::None) => true,
            (O::Boolean(a), O::Boolean(b)) => a == b,
            (O::String(a), O::String(b)) | (O::Label(a), O::Label(b)) => a == b,
            (O::Artifact(a), O::Artifact(b)) => a == b,
            (O::PathFragment(a), O::PathFragment(b)) => a == b,
            (O::Iterable(a), O::Iterable(b)) => a == b,
            (O::NestedSet(a), O::NestedSet(b)) => a == b,
            (O::Structure(a), O::Structure(b)) => Arc::ptr_eq(a, b),
            (O::Value(a), O::Value(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for VariableObject {}

impl Hash for VariableObject {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use VariableObject as O;
        std::mem::discriminant(self).hash(state);
        match self {
            O::Null | O::None => {}
            O::Boolean(b) => b.hash(state),
            O::String(s) | O::Label(s) => s.hash(state),
            O::Artifact(a) => a.hash(state),
            O::PathFragment(p) => p.hash(state),
            O::Iterable(v) => v.hash(state),
            O::NestedSet(n) => n.hash(state),
            O::Structure(s) => (Arc::as_ptr(s) as *const () as usize).hash(state),
            O::Value(v) => v.hash(state),
        }
    }
}

/// Wraps a raw variablesMap value into an appropriate VariableValue if necessary.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:826 CcToolchainVariables.asVariableValue
pub fn as_variable_value(o: &VariableObject) -> Option<VariableValue> {
    match o {
        VariableObject::Null => panic!("NullPointerException"),
        VariableObject::None => None, // null has the same behavior as omitted field
        VariableObject::Boolean(b) => Some(VariableValue::Boolean(BooleanValue::of(*b))),
        VariableObject::String(s) => Some(VariableValue::String(StringValue::new(s.clone()))),
        VariableObject::Label(s) => Some(VariableValue::String(StringValue::new(s.clone()))),
        VariableObject::Artifact(a) => Some(VariableValue::Artifact(ArtifactValue(a.clone()))),
        VariableObject::PathFragment(p) => {
            Some(VariableValue::PathFragment(PathFragmentValue(p.clone())))
        }
        VariableObject::Iterable(v) => {
            Some(VariableValue::Sequence(Sequence { values: v.clone() }))
        }
        VariableObject::NestedSet(n) => Some(VariableValue::Sequence(Sequence {
            values: n.to_list(),
        })),
        VariableObject::Structure(s) => Some(VariableValue::Structure(
            StarlarkStructureAdapter::new(s.clone()),
        )),
        VariableObject::Value(v) => Some(v.clone()),
    }
}

/// Value of a build variable exposed to the CROSSTOOL used for flag expansion.
///
/// `VariableValue` represents either primitive values or an arbitrarily deeply nested recursive
/// structures or sequences.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:483 CcToolchainVariables.VariableValue
// PORT: Java's open interface becomes a closed enum of the Java implementations plus `Custom` for
// other implementations of [`VariableValueAdapter`].
#[derive(Clone, Debug)]
pub enum VariableValue {
    Sequence(Sequence),
    String(StringValue),
    Boolean(BooleanValue),
    Artifact(ArtifactValue),
    PathFragment(PathFragmentValue),
    Structure(StarlarkStructureAdapter),
    Custom(Arc<dyn VariableValueAdapter>),
}

impl PartialEq for VariableValue {
    fn eq(&self, other: &Self) -> bool {
        use VariableValue as V;
        match (self, other) {
            (V::Sequence(a), V::Sequence(b)) => a == b,
            (V::String(a), V::String(b)) => a == b,
            (V::Boolean(a), V::Boolean(b)) => a == b,
            (V::Artifact(a), V::Artifact(b)) => a == b,
            (V::PathFragment(a), V::PathFragment(b)) => a == b,
            (V::Structure(a), V::Structure(b)) => Arc::ptr_eq(&a.val, &b.val),
            (V::Custom(a), V::Custom(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for VariableValue {}

impl Hash for VariableValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        use VariableValue as V;
        std::mem::discriminant(self).hash(state);
        match self {
            V::Sequence(s) => s.values.hash(state),
            V::String(s) => s.value.hash(state),
            V::Boolean(b) => b.value.hash(state),
            V::Artifact(a) => a.0.hash(state),
            V::PathFragment(p) => p.0.hash(state),
            V::Structure(s) => (Arc::as_ptr(&s.val) as *const () as usize).hash(state),
            V::Custom(c) => (Arc::as_ptr(c) as *const () as usize).hash(state),
        }
    }
}

/// Adapter for [`VariableValue`] predefining error handling methods. Override
/// `get_variable_type_name`, `is_truthy`, and one of `get_field_value` or `get_string_value`, and
/// you'll get error handling for the other methods for free.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:530 CcToolchainVariables.VariableValueAdapter
pub trait VariableValueAdapter: Send + Sync + fmt::Debug {
    /// Returns human-readable variable type name to be used in error messages.
    fn get_variable_type_name(&self) -> String;

    /// Returns true if the variable is truthy.
    fn is_truthy(&self) -> bool;

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:537 CcToolchainVariables.VariableValueAdapter.getFieldValue
    fn get_field_value(
        &self,
        variable_name: &str,
        field: &str,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _path_mapper: &dyn PathMapper,
        throw_on_missing_variable: bool,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        default_get_field_value(
            variable_name,
            field,
            &self.get_variable_type_name(),
            throw_on_missing_variable,
        )
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:556 CcToolchainVariables.VariableValueAdapter.getStringValue
    fn get_string_value(
        &self,
        variable_name: &str,
        _path_mapper: &dyn PathMapper,
    ) -> Result<String, ExpansionException> {
        Err(default_get_string_value(
            variable_name,
            &self.get_variable_type_name(),
        ))
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:537 CcToolchainVariables.VariableValueAdapter.getFieldValue
fn default_get_field_value(
    variable_name: &str,
    field: &str,
    type_name: &str,
    throw_on_missing_variable: bool,
) -> Result<Option<VariableValue>, ExpansionException> {
    if throw_on_missing_variable {
        Err(ExpansionException::new(format!(
            "Invalid toolchain configuration: Cannot expand variable '{}.{}': variable '{}' is \
             {}, expected structure",
            variable_name, field, variable_name, type_name
        )))
    } else {
        Ok(None)
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:556 CcToolchainVariables.VariableValueAdapter.getStringValue
fn default_get_string_value(variable_name: &str, type_name: &str) -> ExpansionException {
    ExpansionException::new(format!(
        "Invalid toolchain configuration: Cannot expand variable '{}': expected string, found {}",
        variable_name, type_name
    ))
}

impl VariableValue {
    /// Returns human-readable variable type name to be used in error messages.
    pub fn get_variable_type_name(&self) -> String {
        match self {
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:595 CcToolchainVariables.Sequence.getVariableTypeName
            VariableValue::Sequence(_) => "sequence".to_string(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:624 CcToolchainVariables.StringValue.getVariableTypeName
            VariableValue::String(_) => "string".to_string(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:651 CcToolchainVariables.BooleanValue.getVariableTypeName
            VariableValue::Boolean(_) => "boolean".to_string(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:678 CcToolchainVariables.ArtifactValue.getVariableTypeName
            VariableValue::Artifact(_) => "artifact".to_string(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:700 CcToolchainVariables.PathFragmentValue.getVariableTypeName
            VariableValue::PathFragment(_) => "pathfragment".to_string(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:907 CcToolchainVariables.StarlarkStructureAdapter.getVariableTypeName
            VariableValue::Structure(s) => s.val.starlark_type(),
            VariableValue::Custom(c) => c.get_variable_type_name(),
        }
    }

    /// Returns string value of the variable, if the variable type can be converted to string
    /// (e.g. StringValue), or an error if it cannot (e.g. Sequence).
    pub fn get_string_value(
        &self,
        variable_name: &str,
        path_mapper: &dyn PathMapper,
    ) -> Result<String, ExpansionException> {
        match self {
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:619 CcToolchainVariables.StringValue.getStringValue
            VariableValue::String(s) => Ok(path_mapper.map_heuristically(&s.value)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:646 CcToolchainVariables.BooleanValue.getStringValue
            VariableValue::Boolean(b) => Ok(if b.value { "1" } else { "0" }.to_string()),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:673 CcToolchainVariables.ArtifactValue.getStringValue
            VariableValue::Artifact(a) => Ok(path_mapper.get_mapped_exec_path_string(&*a.0)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:695 CcToolchainVariables.PathFragmentValue.getStringValue
            VariableValue::PathFragment(p) => {
                Ok(path_mapper.map(&p.0).get_safe_path_string().to_string())
            }
            VariableValue::Custom(c) => c.get_string_value(variable_name, path_mapper),
            VariableValue::Sequence(_) | VariableValue::Structure(_) => Err(
                default_get_string_value(variable_name, &self.get_variable_type_name()),
            ),
        }
    }

    /// Returns value of the field, if the variable is of struct type or an error if it is not or
    /// no such field exists.
    pub fn get_field_value(
        &self,
        variable_name: &str,
        field: &str,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        throw_on_missing_variable: bool,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        match self {
            VariableValue::Structure(s) => s.get_field_value(
                variable_name,
                field,
                input_metadata_provider,
                path_mapper,
                throw_on_missing_variable,
            ),
            VariableValue::Custom(c) => c.get_field_value(
                variable_name,
                field,
                input_metadata_provider,
                path_mapper,
                throw_on_missing_variable,
            ),
            _ => default_get_field_value(
                variable_name,
                field,
                &self.get_variable_type_name(),
                throw_on_missing_variable,
            ),
        }
    }

    /// Java's `@VisibleForTesting getFieldValue(variableName, field)`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:510 CcToolchainVariables.VariableValue.getFieldValue
    pub fn get_field_value_for_testing(
        &self,
        variable_name: &str,
        field: &str,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        self.get_field_value(variable_name, field, None, &NOOP, true)
    }

    /// Returns true if the variable is truthy.
    pub fn is_truthy(&self) -> bool {
        match self {
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:600 CcToolchainVariables.Sequence.isTruthy
            VariableValue::Sequence(s) => !s.values.is_empty(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:629 CcToolchainVariables.StringValue.isTruthy
            VariableValue::String(s) => !s.value.is_empty(),
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:656 CcToolchainVariables.BooleanValue.isTruthy
            VariableValue::Boolean(b) => b.value,
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:683 CcToolchainVariables.ArtifactValue.isTruthy
            VariableValue::Artifact(_) => true,
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:705 CcToolchainVariables.PathFragmentValue.isTruthy
            VariableValue::PathFragment(_) => true,
            // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:912 CcToolchainVariables.StarlarkStructureAdapter.isTruthy
            VariableValue::Structure(s) => s.val.truth(),
            VariableValue::Custom(c) => c.is_truthy(),
        }
    }
}

/// Returns Iterable value of the variable, if the variable type can be converted to a Iterable
/// (e.g. Sequence), or an error if it cannot (e.g. StringValue).
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:572 CcToolchainVariables.getSequenceValue
pub fn get_sequence_value(
    variable_name: &str,
    value: &VariableValue,
) -> Result<Vec<Option<VariableValue>>, ExpansionException> {
    if let VariableValue::Sequence(seq) = value {
        return Ok(seq.get_sequence_value());
    }
    Err(ExpansionException::new(format!(
        "Invalid toolchain configuration: Cannot expand variable '{}': expected sequence, found {}",
        variable_name,
        value.get_variable_type_name()
    )))
}

/// Sequence of arbitrary VariableValue objects.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:587 CcToolchainVariables.Sequence
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    values: Arc<Vec<VariableObject>>,
}

impl Sequence {
    pub fn new(values: Vec<VariableObject>) -> Sequence {
        Sequence {
            values: Arc::new(values),
        }
    }

    pub fn values(&self) -> &[VariableObject] {
        &self.values
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:590 CcToolchainVariables.Sequence.getSequenceValue
    // PORT: `None` where Java's transform yields null (a `None` element).
    pub fn get_sequence_value(&self) -> Vec<Option<VariableValue>> {
        self.values.iter().map(as_variable_value).collect()
    }
}

/// Most leaves in the variable sequence node tree are simple string values.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:611 CcToolchainVariables.StringValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringValue {
    value: Arc<str>,
}

impl StringValue {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:614 CcToolchainVariables.StringValue.<init>
    pub fn new(value: impl Into<Arc<str>>) -> StringValue {
        StringValue {
            value: value.into(),
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:637 CcToolchainVariables.BooleanValue
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BooleanValue {
    value: bool,
}

impl BooleanValue {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:641 CcToolchainVariables.BooleanValue.of
    pub fn of(value: bool) -> BooleanValue {
        BooleanValue { value }
    }
}

/// Represents leaves in the variable sequence node tree that are paths of artifacts.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:669 CcToolchainVariables.ArtifactValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactValue(pub ArtifactRef);

// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:691 CcToolchainVariables.PathFragmentValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathFragmentValue(pub PathFragment);

/// Adapts Starlark structures.
///
/// It's used to support NamedLibraryInfo, ObjectFileGroupInfo and VersionedLibraryInfo
/// structures create in `create_libraries_to_link_values.bzl`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:848 CcToolchainVariables.StarlarkStructureAdapter
#[derive(Clone, Debug)]
pub struct StarlarkStructureAdapter {
    val: Arc<dyn Structure>,
}

impl StarlarkStructureAdapter {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:851 CcToolchainVariables.StarlarkStructureAdapter.<init>
    pub fn new(val: Arc<dyn Structure>) -> StarlarkStructureAdapter {
        StarlarkStructureAdapter { val }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:857 CcToolchainVariables.StarlarkStructureAdapter.getFieldValue
    fn get_field_value(
        &self,
        variable_name: &str,
        field: &str,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
        throw_on_missing_variable: bool,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        match self.val.get_value(field) {
            Ok(field_value) => {
                // Special handling for tree artifacts. Needed for ObjectFileGroupInfo containing
                // a tree artifact. When this code is migrated to Starlark, the expansion should
                // happen on Starlark command lines.
                if let Some(VariableObject::Iterable(iterable)) = &field_value {
                    let mut expanded_iterable = Vec::new();
                    for element in iterable.iter() {
                        if let (VariableObject::Artifact(artifact), Some(imp)) =
                            (element, input_metadata_provider)
                            && artifact.is_tree_artifact()
                        {
                            if let Some(children) = imp.get_tree_metadata(artifact) {
                                for child in children {
                                    expanded_iterable.push(VariableObject::string(
                                        &path_mapper.get_mapped_exec_path_string(&*child.0),
                                    ));
                                }
                            }
                        } else {
                            expanded_iterable.push(element.clone());
                        }
                    }
                    return Ok(Some(VariableValue::Sequence(Sequence::new(
                        expanded_iterable,
                    ))));
                }
                match field_value {
                    None => Ok(None),
                    Some(v) => Ok(as_variable_value(&v)),
                }
            }
            Err(_e) => {
                if throw_on_missing_variable {
                    Err(ExpansionException::new(format!(
                        "Invalid toolchain configuration: Cannot expand variable '{}.{}': \
                         variable '{}' is {}, expected structure",
                        variable_name,
                        field,
                        variable_name,
                        self.val.starlark_type()
                    )))
                } else {
                    Ok(None)
                }
            }
        }
    }
}

/// Cached result of a structured lookup: Java's `VariableValue`, `String` error message or
/// `NULL_MARKER`.
#[derive(Clone, Debug)]
enum CachedVariable {
    Value(VariableValue),
    Error(String),
    Null,
}

/// Configured build variables usable by the toolchain configuration.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:65 CcToolchainVariables
pub struct CcToolchainVariables {
    kind: VariablesKind,
    // Values in this cache are either VariableValue, String error message, or NULL_MARKER.
    //
    // It is initialized lazily.
    structured_variable_cache: Mutex<Option<HashMap<String, CachedVariable>>>,
}

enum VariablesKind {
    Map(MapVariables),
    Single(SingleVariables),
}

/// Java: `CcToolchainVariables.MapVariables`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:917 CcToolchainVariables.MapVariables
struct MapVariables {
    parent: Option<CcToolchainVariablesRef>,
    /// Maps from a variable name to the index of the corresponding value in values (sorted by
    /// name).
    key_to_index: IndexMap<String, usize>,
    /// The values belonging to the keys stored in keyToIndex.
    values: Vec<VariableObject>,
}

/// Java: `CcToolchainVariables.SingleVariables`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1014 CcToolchainVariables.SingleVariables
struct SingleVariables {
    parent: Option<CcToolchainVariablesRef>,
    name: String,
    variable_value: VariableObject,
}

impl fmt::Debug for CcToolchainVariables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VariablesKind::Map(m) => f
                .debug_struct("MapVariables")
                .field("parent", &m.parent)
                .field("keys", &m.key_to_index.keys().collect::<Vec<_>>())
                .field("values", &m.values)
                .finish(),
            VariablesKind::Single(s) => f
                .debug_struct("SingleVariables")
                .field("parent", &s.parent)
                .field("name", &s.name)
                .field("value", &s.variable_value)
                .finish(),
        }
    }
}

impl PartialEq for CcToolchainVariables {
    /// NB: this compares parents using reference equality instead of logical equality, as Java.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:994 CcToolchainVariables.MapVariables.equals
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1045 CcToolchainVariables.SingleVariables.equals
    fn eq(&self, other: &Self) -> bool {
        fn same_parent(
            a: &Option<CcToolchainVariablesRef>,
            b: &Option<CcToolchainVariablesRef>,
        ) -> bool {
            match (a, b) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
        }
        if std::ptr::eq(self, other) {
            return true;
        }
        match (&self.kind, &other.kind) {
            (VariablesKind::Map(a), VariablesKind::Map(b)) => {
                same_parent(&a.parent, &b.parent)
                    && a.key_to_index == b.key_to_index
                    && a.values == b.values
            }
            (VariablesKind::Single(a), VariablesKind::Single(b)) => {
                same_parent(&a.parent, &b.parent)
                    && a.name == b.name
                    && a.variable_value == b.variable_value
            }
            _ => false,
        }
    }
}

impl CcToolchainVariables {
    fn from_kind(kind: VariablesKind) -> CcToolchainVariables {
        CcToolchainVariables {
            kind,
            structured_variable_cache: Mutex::new(None),
        }
    }

    /// Returns an empty variables instance.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:273 CcToolchainVariables.empty
    pub fn empty() -> CcToolchainVariablesRef {
        static EMPTY: std::sync::LazyLock<CcToolchainVariablesRef> =
            std::sync::LazyLock::new(|| CcToolchainVariables::builder().build());
        EMPTY.clone()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:710 CcToolchainVariables.builder
    pub fn builder() -> CcToolchainVariablesBuilder {
        CcToolchainVariablesBuilder::new(None)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:714 CcToolchainVariables.builder
    pub fn builder_with_parent(
        parent: Option<CcToolchainVariablesRef>,
    ) -> CcToolchainVariablesBuilder {
        CcToolchainVariablesBuilder::new(parent)
    }

    /// Java: `new MapVariables(parent, variablesMap)`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:935 CcToolchainVariables.MapVariables.<init>
    pub fn new_map_variables(
        parent: Option<CcToolchainVariablesRef>,
        variables_map: &IndexMap<String, VariableObject>,
    ) -> CcToolchainVariablesRef {
        let mut keys: Vec<&String> = variables_map.keys().collect();
        // ImmutableList.sortedCopyOf: natural String order.
        // PORT: Rust byte order equals Java's UTF-16 order for BMP-only strings.
        keys.sort();
        let mut key_to_index = IndexMap::new();
        let mut values = Vec::new();
        for (index, key) in keys.into_iter().enumerate() {
            key_to_index.insert(key.clone(), index);
            // PORT: Depset values are unwrapped by the Starlark layer before they reach here.
            values.push(variables_map[key].clone());
        }
        Arc::new(CcToolchainVariables::from_kind(VariablesKind::Map(
            MapVariables {
                parent,
                key_to_index,
                values,
            },
        )))
    }

    /// Java: `new SingleVariables(parent, name, variableValue)`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1019 CcToolchainVariables.SingleVariables.<init>
    pub fn new_single_variables(
        parent: Option<CcToolchainVariablesRef>,
        name: &str,
        variable_value: VariableObject,
    ) -> CcToolchainVariablesRef {
        Arc::new(CcToolchainVariables::from_kind(VariablesKind::Single(
            SingleVariables {
                parent,
                name: name.to_string(),
                variable_value,
            },
        )))
    }

    /// Retrieves a string sequence variable named `variable_name` from `variables` and converts
    /// it into a list of plain strings.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:305 CcToolchainVariables.toStringList
    pub fn to_string_list(
        variables: &CcToolchainVariables,
        variable_name: &str,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, ExpansionException> {
        let mut result = Vec::new();
        for value in get_sequence_value(
            variable_name,
            &variables.get_variable(variable_name, path_mapper)?,
        )? {
            let value = value.unwrap_or_else(|| panic!("NullPointerException"));
            result.push(value.get_string_value(variable_name, path_mapper)?);
        }
        Ok(result)
    }

    /// Gets a variable value named `name`. Supports accessing fields in structures (e.g.
    /// 'libraries_to_link.interface_libraries').
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:323 CcToolchainVariables.getVariable
    pub fn get_variable(
        &self,
        name: &str,
        path_mapper: &dyn PathMapper,
    ) -> Result<VariableValue, ExpansionException> {
        self.lookup_variable(name, true, None, path_mapper)
            .map(|v| v.expect("lookupVariable returned null while asked to throw"))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:328 CcToolchainVariables.getVariable
    pub fn get_variable_with_metadata(
        &self,
        name: &str,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<VariableValue, ExpansionException> {
        self.lookup_variable(name, true, input_metadata_provider, path_mapper)
            .map(|v| v.expect("lookupVariable returned null while asked to throw"))
    }

    /// Looks up a variable named `name` or returns a reason why the variable was not found.
    /// Supports accessing fields in structures.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:340 CcToolchainVariables.lookupVariable
    fn lookup_variable(
        &self,
        name: &str,
        throw_on_missing_variable: bool,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        if let Some(var) = self.get_non_structured_variable(name) {
            return Ok(Some(var));
        }

        if !name.contains('.') {
            if throw_on_missing_variable {
                return Err(ExpansionException::new(format!(
                    "Invalid toolchain configuration: Cannot find variable named '{}'.",
                    name
                )));
            }
            return Ok(None);
        }

        let cached = self
            .structured_variable_cache
            .lock()
            .get_or_insert_with(HashMap::new)
            .get(name)
            .cloned();
        let variable_or_error = match cached {
            Some(v) => v,
            None => {
                // PORT: computed outside the lock; Java computes outside any lock too.
                let variable_or_error = match self.get_structure_variable(
                    name,
                    throw_on_missing_variable,
                    input_metadata_provider,
                    path_mapper,
                ) {
                    Ok(Some(variable)) => CachedVariable::Value(variable),
                    Ok(None) => CachedVariable::Null,
                    Err(e) => {
                        if throw_on_missing_variable {
                            CachedVariable::Error(e.message)
                        } else {
                            panic!(
                                "IllegalStateException: Should not happen - call to \
                                 getStructuredVariable threw when asked not to."
                            )
                        }
                    }
                };
                self.structured_variable_cache
                    .lock()
                    .get_or_insert_with(HashMap::new)
                    .entry(name.to_string())
                    .or_insert_with(|| variable_or_error.clone());
                variable_or_error
            }
        };

        if let CachedVariable::Value(variable_value) = variable_or_error {
            return Ok(Some(variable_value));
        }
        if throw_on_missing_variable {
            return Err(ExpansionException::new(match variable_or_error {
                CachedVariable::Error(string) => string,
                _ => format!(
                    "Invalid toolchain configuration: Cannot find variable named '{}'.",
                    name
                ),
            }));
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:399 CcToolchainVariables.getStructureVariable
    fn get_structure_variable(
        &self,
        name: &str,
        throw_on_missing_variable: bool,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        if !name.contains('.') {
            return Ok(None);
        }

        let mut fields_to_access: Vec<String> = Vec::new();
        let mut struct_path = name.to_string();
        let mut variable;

        loop {
            let last_dot = struct_path.rfind('.').unwrap();
            fields_to_access.push(struct_path[last_dot + 1..].to_string());
            struct_path = struct_path[..last_dot].to_string();
            variable = self.get_non_structured_variable(&struct_path);
            if !(variable.is_none() && struct_path.contains('.')) {
                break;
            }
        }

        let Some(mut variable) = variable else {
            return Ok(None);
        };

        while let Some(field) = fields_to_access.pop() {
            match variable.get_field_value(
                &struct_path,
                &field,
                input_metadata_provider,
                path_mapper,
                throw_on_missing_variable,
            )? {
                Some(v) => variable = v,
                None => {
                    if throw_on_missing_variable {
                        return Err(ExpansionException::new(format!(
                            "Invalid toolchain configuration: Cannot expand variable '{}.{}': \
                             structure {} doesn't have a field named '{}'",
                            struct_path, field, struct_path, field
                        )));
                    } else {
                        return Ok(None);
                    }
                }
            }
        }
        Ok(Some(variable))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:443 CcToolchainVariables.getStringVariable
    pub fn get_string_variable(
        &self,
        variable_name: &str,
        path_mapper: &dyn PathMapper,
    ) -> Result<String, ExpansionException> {
        self.get_variable_with_metadata(variable_name, None, path_mapper)?
            .get_string_value(variable_name, path_mapper)
    }

    /// Returns whether `variable` is set.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:450 CcToolchainVariables.isAvailable
    pub fn is_available(&self, variable: &str) -> bool {
        self.is_available_with_metadata(variable, None)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:454 CcToolchainVariables.isAvailable
    pub fn is_available_with_metadata(
        &self,
        variable: &str,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
    ) -> bool {
        // Availability doesn't depend on the path mapper.
        match self.lookup_variable(variable, false, input_metadata_provider, &NOOP) {
            Ok(v) => v.is_some(),
            Err(_) => panic!(
                "IllegalStateException: Should not happen - call to lookupVariable threw when \
                 asked not to."
            ),
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:958 CcToolchainVariables.MapVariables.getVariableKeys
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1026 CcToolchainVariables.SingleVariables.getVariableKeys
    pub fn get_variable_keys(&self) -> Vec<String> {
        match &self.kind {
            VariablesKind::Map(m) => m.key_to_index.keys().cloned().collect(),
            VariablesKind::Single(s) => vec![s.name.clone()],
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:963 CcToolchainVariables.MapVariables.addVariablesToMap
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1031 CcToolchainVariables.SingleVariables.addVariablesToMap
    fn add_variables_to_map(&self, variables_map: &mut IndexMap<String, VariableObject>) {
        match &self.kind {
            VariablesKind::Map(m) => {
                for (key, &index) in &m.key_to_index {
                    variables_map.insert(key.clone(), m.values[index].clone());
                }
            }
            VariablesKind::Single(s) => {
                variables_map.insert(s.name.clone(), s.variable_value.clone());
            }
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:971 CcToolchainVariables.MapVariables.getNonStructuredVariable
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:1037 CcToolchainVariables.SingleVariables.getNonStructuredVariable
    fn get_non_structured_variable(&self, name: &str) -> Option<VariableValue> {
        match &self.kind {
            VariablesKind::Map(m) => {
                if let Some(&index) = m.key_to_index.get(name) {
                    return as_variable_value(&m.values[index]);
                }
                if let Some(parent) = &m.parent {
                    return parent.get_non_structured_variable(name);
                }
                None
            }
            VariablesKind::Single(s) => {
                if s.name == name {
                    return as_variable_value(&s.variable_value);
                }
                match &s.parent {
                    None => None,
                    Some(parent) => parent.get_non_structured_variable(name),
                }
            }
        }
    }
}

/// Builder for `Variables`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:720 CcToolchainVariables.Builder
// PORT: the weak interners (string sequences, keys) are memory optimizations and are skipped.
pub struct CcToolchainVariablesBuilder {
    variables_map: IndexMap<String, VariableObject>,
    parent: Option<CcToolchainVariablesRef>,
}

impl CcToolchainVariablesBuilder {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:724 CcToolchainVariables.Builder.<init>
    fn new(parent: Option<CcToolchainVariablesRef>) -> CcToolchainVariablesBuilder {
        CcToolchainVariablesBuilder {
            variables_map: IndexMap::new(),
            parent,
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:730 CcToolchainVariables.Builder.overrideVariable
    pub fn override_variable(&mut self, name: &str, value: ArtifactRef) -> &mut Self {
        self.variables_map
            .insert(name.to_string(), VariableObject::Artifact(value));
        self
    }

    /// Add a sequence variable that expands `name` to `values`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:746 CcToolchainVariables.Builder.addStringSequenceVariable
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:764 CcToolchainVariables.Builder.addStringSequenceVariable
    pub fn add_string_sequence_variable<S: AsRef<str>>(
        &mut self,
        name: &str,
        values: impl IntoIterator<Item = S>,
    ) -> &mut Self {
        self.check_variable_not_present_already(name);
        self.variables_map
            .insert(name.to_string(), VariableObject::string_list(values));
        self
    }

    /// Adds a variable that expands `name` to the `value`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:774 CcToolchainVariables.Builder.addVariable
    pub fn add_variable(&mut self, name: &str, value: VariableObject) -> &mut Self {
        self.check_variable_not_present_already(name);
        self.variables_map.insert(name.to_string(), value);
        self
    }

    /// Add all string variables in a map.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:783 CcToolchainVariables.Builder.addAllStringVariables
    pub fn add_all_string_variables(&mut self, variables: &IndexMap<String, String>) -> &mut Self {
        for name in variables.keys() {
            self.check_variable_not_present_already(name);
        }
        for (name, value) in variables {
            self.variables_map
                .insert(name.clone(), VariableObject::string(value));
        }
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:791 CcToolchainVariables.Builder.checkVariableNotPresentAlready
    fn check_variable_not_present_already(&self, name: &str) {
        blaze_util::check_argument!(
            !self.variables_map.contains_key(name),
            "Cannot overwrite variable '{}'",
            name
        );
    }

    /// Adds all variables to this builder. Cannot override already added variables. Does not add
    /// variables defined in the `parent` variables.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:802 CcToolchainVariables.Builder.addAllNonTransitive
    pub fn add_all_non_transitive(&mut self, variables: &CcToolchainVariables) -> &mut Self {
        let intersection: Vec<String> = variables
            .get_variable_keys()
            .into_iter()
            .filter(|k| self.variables_map.contains_key(k))
            .collect();
        blaze_util::check_argument!(
            intersection.is_empty(),
            "Cannot overwrite existing variables: {}",
            JavaList(&intersection)
        );
        variables.add_variables_to_map(&mut self.variables_map);
        self
    }

    /// Returns a new [`CcToolchainVariables`] object.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainVariables.java:814 CcToolchainVariables.Builder.build
    pub fn build(&self) -> CcToolchainVariablesRef {
        if self.variables_map.len() == 1 {
            let (name, value) = self.variables_map.get_index(0).unwrap();
            return CcToolchainVariables::new_single_variables(
                self.parent.clone(),
                name,
                value.clone(),
            );
        }
        CcToolchainVariables::new_map_variables(self.parent.clone(), &self.variables_map)
    }
}
