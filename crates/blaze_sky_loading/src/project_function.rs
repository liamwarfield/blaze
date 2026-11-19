//! Project files (`PROJECT.scl`): ports of `skyframe/ProjectFilesLookupFunction.java`,
//! `skyframe/ProjectFilesLookupValue.java`, `skyframe/ProjectFunction.java` and
//! `skyframe/ProjectValue.java`.
//!
//! PORT: the PROJECT_FILES_LOOKUP port moved here from `blaze_sky_top::project` so that
//! BUILD_OPTIONS_SCOPE (`blaze_sky_analysis`) can request it; `blaze_sky_top::project` re-exports
//! it.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_skyframe::{
    ComputeError, Environment, EvaluatorBuilder, Key, SkyFunctionException, SkyFunctionName,
    Transience,
};
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use starlark::values::Value;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;

use crate::actions::FileValue;
use crate::bzl_load::{BzlLoadFailedException, BzlLoadKey};
use crate::containing_package_lookup::ContainingPackageLookupKey;
use crate::file::FileKey;
use crate::io::IOException;

/// Name of project metadata files. See `Project`.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:35 ProjectFilesLookupFunction.PROJECT_FILE_NAME
pub const PROJECT_FILE_NAME: &str = "PROJECT.scl";

/// `SkyFunctions.PROJECT_FILES_LOOKUP`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:45 SkyFunctions.PROJECT_FILES_LOOKUP
pub const PROJECT_FILES_LOOKUP: SkyFunctionName = SkyFunctionName("PROJECT_FILES_LOOKUP");

/// `SkyFunctions.PROJECT`.
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:44 SkyFunctions.PROJECT
pub const PROJECT: SkyFunctionName = SkyFunctionName("PROJECT");

/// The key of a `ProjectFilesLookupValue`: a package.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:77 ProjectFilesLookupValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectFilesLookupKey(pub PackageIdentifier);

impl ProjectFilesLookupKey {
    // Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:59 ProjectFilesLookupValue.key
    pub fn key(id: PackageIdentifier) -> Self {
        ProjectFilesLookupKey(id)
    }
}

impl fmt::Debug for ProjectFilesLookupKey {
    // Java: com/google/devtools/build/skyframe/AbstractSkyKey.java:59 AbstractSkyKey.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", PROJECT_FILES_LOOKUP, self.0)
    }
}

impl Key for ProjectFilesLookupKey {
    type Value = ProjectFilesLookupValue;
    const FUNCTION: SkyFunctionName = PROJECT_FILES_LOOKUP;
}

/// The project files of a package: every `PROJECT.scl` in the package and in the packages above
/// it.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:33 ProjectFilesLookupValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFilesLookupValue {
    project_files: Vec<Label>,
}

impl ProjectFilesLookupValue {
    /// Returns the project files for the corresponding key's package.
    ///
    /// Order is innermost to outermost: if both `a/PROJECT.scl` and `a/b/c/PROJECT.scl` are
    /// included, `a/b/c/PROJECT.scl` appears first.
    // Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupValue.java:49 ProjectFilesLookupValue.getProjectFiles
    pub fn get_project_files(&self) -> &[Label] {
        &self.project_files
    }
}

/// A project file exists but isn't a file.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:118 ProjectFilesLookupFunction.UnexpectedProjectFileTypeException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{0}")]
pub struct UnexpectedProjectFileTypeException(pub String);

/// `SkyFunction` for `ProjectFilesLookupValue`.
///
/// PORT: Java keeps the directory being examined and the files found so far in a
/// `SkyKeyComputeState`, to go on where it stopped after a restart. A dependency request blocks
/// here, so the loop runs through; a second run (error bubbling) starts again from the key's
/// package and asks for the same dependencies.
// Java: com/google/devtools/build/lib/skyframe/ProjectFilesLookupFunction.java:46 ProjectFilesLookupFunction.compute
fn project_files_lookup_function(
    key: &ProjectFilesLookupKey,
    env: &mut Environment<'_>,
) -> Result<Option<ProjectFilesLookupValue>, ComputeError> {
    // Which project files have we discovered so far?
    let mut project_files: Vec<Label> = Vec::new();
    // Tracks the current directory we're checking for BUILD and PROJECT.scl files. According to
    // the ProjectFilesLookupValue API contract, the original value from the skykey should be a
    // package (i.e. has a BUILD file). But this code doesn't require that: if the directory
    // doesn't have a BUILD file the code still gracefully finds the nearest enclosing package.
    // This is especially important when walking up the directory path, since the parent directory
    // of a package isn't necessarily another package.
    let mut current_dir: PackageIdentifier = key.0.clone();
    loop {
        let Some(innermost_pkg_lookup_value) =
            env.get_value(ContainingPackageLookupKey(current_dir.clone()))?
        else {
            return Ok(None);
        };
        if !innermost_pkg_lookup_value.has_containing_package() {
            // We've reached the root directory: nothing left. This list may be empty but that's
            // okay. That just means the input package has no associated project files.
            return Ok(Some(ProjectFilesLookupValue { project_files }));
        }
        // Now that we've found a BUILD file, determine the project file path to look for.
        let innermost_pkg_id = innermost_pkg_lookup_value
            .get_containing_package_name()
            .clone();
        let project_file_label =
            match Label::create_in_package(&innermost_pkg_id, PROJECT_FILE_NAME) {
                Ok(label) => label,
                Err(e) => panic!(
                    "IllegalStateException: Unexpected failure parsing {}: {}",
                    PROJECT_FILE_NAME, e
                ),
            };
        // Lookup the project file.
        let project_file_path = innermost_pkg_id
            .get_package_fragment()
            .get_relative_str(PROJECT_FILE_NAME);
        let file_sky_key = FileKey::key(RootedPath::to_rooted_path(
            innermost_pkg_lookup_value
                .get_containing_package_root()
                .clone(),
            &project_file_path,
        ));
        let file_value: Option<Arc<FileValue>> =
            match env.get_value_or_throw::<FileKey, IOException>(file_sky_key)? {
                Ok(v) => v,
                // ProjectFilesLookupException(IOException)
                Err(e) => return Err(SkyFunctionException::persistent(e).into()),
            };
        let Some(file_value) = file_value else {
            return Ok(None);
        };
        if file_value.is_file() {
            project_files.push(project_file_label);
        } else if file_value.exists() {
            // ProjectFilesLookupException(UnexpectedProjectFileTypeException)
            return Err(
                SkyFunctionException::persistent(UnexpectedProjectFileTypeException(format!(
                    "{} isn't a file",
                    project_file_path.get_path_string()
                )))
                .into(),
            );
        }
        let Some(parent_dir) = innermost_pkg_id
            .get_package_fragment()
            .get_parent_directory()
        else {
            // Hit the root directory. Returns the results we've collected.
            return Ok(Some(ProjectFilesLookupValue { project_files }));
        };
        current_dir = PackageIdentifier::create(current_dir.get_repository().clone(), parent_dir);
    }
}

// -------------------------------------------------------------------------------------------
// ProjectValue

/// The enforcement policy of a project file.
// Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:36 ProjectValue.EnforcementPolicy
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum EnforcementPolicy {
    /// Default, enforced in ProjectFunction#compute.
    WARN,
    COMPATIBLE,
    STRICT,
}

impl EnforcementPolicy {
    fn value(self) -> &'static str {
        match self {
            EnforcementPolicy::WARN => "warn",
            EnforcementPolicy::COMPATIBLE => "compatible",
            EnforcementPolicy::STRICT => "strict",
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:47 ProjectValue.EnforcementPolicy.fromString
    /// `Err` is the `IllegalArgumentException` message.
    pub fn from_string(value: &str) -> Result<EnforcementPolicy, String> {
        for policy in [
            EnforcementPolicy::WARN,
            EnforcementPolicy::COMPATIBLE,
            EnforcementPolicy::STRICT,
        ] {
            if policy.value() == value {
                return Ok(policy);
            }
        }
        Err(format!("invalid enforcement_policy '{}'", value))
    }
}

/// A buildable unit of a project: a named set of flags, optionally restricted to target
/// patterns.
// Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:74 ProjectValue.BuildableUnit
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildableUnit {
    name: String,
    target_pattern_matcher: crate::simple_target_pattern_matcher::SimpleTargetPatternMatcher,
    description: String,
    flags: Vec<String>,
    is_default: bool,
}

impl BuildableUnit {
    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:75 ProjectValue.BuildableUnit.create
    pub fn create(
        name: String,
        target_patterns: Vec<String>,
        description: String,
        flags: Vec<String>,
        is_default: bool,
    ) -> Result<BuildableUnit, blaze_cmdline::LabelSyntaxException> {
        Ok(BuildableUnit {
            name,
            target_pattern_matcher:
                crate::simple_target_pattern_matcher::SimpleTargetPatternMatcher::create(
                    &target_patterns,
                )?,
            description,
            flags,
            is_default,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn target_pattern_matcher(
        &self,
    ) -> &crate::simple_target_pattern_matcher::SimpleTargetPatternMatcher {
        &self.target_pattern_matcher
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn flags(&self) -> &[String] {
        &self.flags
    }

    pub fn is_default(&self) -> bool {
        self.is_default
    }
}

/// A project file's content.
// Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:31 ProjectValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectValue {
    enforcement_policy: EnforcementPolicy,
    /// `ImmutableMap<String, Collection<String>>` (insertion order).
    project_directories: IndexMap<String, Vec<String>>,
    buildable_units: Option<IndexMap<String, BuildableUnit>>,
    always_allowed_configs: Option<Vec<String>>,
    actual_project_file: Option<Label>,
}

impl ProjectValue {
    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:98 ProjectValue.<init>
    pub fn new(
        enforcement_policy: EnforcementPolicy,
        project_directories: IndexMap<String, Vec<String>>,
        buildable_units: Option<IndexMap<String, BuildableUnit>>,
        always_allowed_configs: Option<Vec<String>>,
        actual_project_file: Option<Label>,
    ) -> Self {
        ProjectValue {
            enforcement_policy,
            project_directories,
            buildable_units,
            always_allowed_configs,
            actual_project_file,
        }
    }

    /// Returns the default project directories (`project_directories["default"]`).
    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:112 ProjectValue.getDefaultProjectDirectories
    pub fn get_default_project_directories(&self) -> indexmap::IndexSet<String> {
        if self.project_directories.is_empty() {
            return indexmap::IndexSet::new();
        }
        // TODO: b/409377907 - Make sure this check still makes sense with the new format.
        blaze_util::check_argument!(
            self.project_directories.contains_key("default"),
            "project_directories must contain the 'default' key"
        );
        self.project_directories["default"]
            .iter()
            .cloned()
            .collect()
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:124 ProjectValue.getActualProjectFile
    pub fn get_actual_project_file(&self) -> Option<&Label> {
        self.actual_project_file.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:128 ProjectValue.getEnforcementPolicy
    pub fn get_enforcement_policy(&self) -> EnforcementPolicy {
        self.enforcement_policy
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:136 ProjectValue.getBuildableUnits
    pub fn get_buildable_units(&self) -> Option<&IndexMap<String, BuildableUnit>> {
        self.buildable_units.as_ref()
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:145 ProjectValue.getAlwaysAllowedConfigs
    pub fn get_always_allowed_configs(&self) -> Option<&[String]> {
        self.always_allowed_configs.as_deref()
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:150 ProjectValue.getProjectDirectories
    pub fn get_project_directories(&self) -> &IndexMap<String, Vec<String>> {
        &self.project_directories
    }
}

/// The key of a `ProjectValue`: the project file.
// Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:155 ProjectValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ProjectKey {
    project_file: Label,
}

impl ProjectKey {
    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:158 ProjectValue.Key.<init>
    pub fn new(project_file: Label) -> Self {
        ProjectKey { project_file }
    }

    // Java: com/google/devtools/build/lib/skyframe/ProjectValue.java:162 ProjectValue.Key.getProjectFile
    pub fn get_project_file(&self) -> &Label {
        &self.project_file
    }
}

impl fmt::Debug for ProjectKey {
    // PORT: Java's Key has no toString (Object.toString); the label identifies it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProjectValue.Key{{projectFile={}}}", self.project_file)
    }
}

impl Key for ProjectKey {
    type Value = ProjectValue;
    const FUNCTION: SkyFunctionName = PROJECT;
}

// -------------------------------------------------------------------------------------------
// ProjectFunction

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:61 ProjectFunction.ENFORCEMENT_POLICY
const ENFORCEMENT_POLICY: &str = "enforcement_policy";

/// `ReservedGlobals.PROJECT.getKey()`.
// Java: com/google/devtools/build/lib/analysis/Project.java ReservedGlobals.PROJECT
const PROJECT_GLOBAL: &str = "project";

/// The exception a PROJECT node fails with: the message of its cause
/// (`TypecheckFailureException`, `ActiveDirectoriesException`, `BadProjectFileException`,
/// `LabelSyntaxException` or `BzlLoadFailedException`).
// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:440 ProjectFunction.ProjectFunctionException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProjectFunctionException {
    pub message: String,
}

fn project_function_exception(message: String) -> ComputeError {
    SkyFunctionException::new(ProjectFunctionException { message }, Transience::Persistent).into()
}

/// Java `Object.getClass()` of a Starlark value, as printed in the error messages.
///
/// PORT: Java prints the class of the Java object; this maps the Starlark types to the classes
/// that represent them in Java.
fn java_class_of(v: Value) -> String {
    let class = match v.get_type() {
        "string" => "java.lang.String",
        "int" => "net.starlark.java.eval.StarlarkInt$Int32",
        "bool" => "java.lang.Boolean",
        "float" => "net.starlark.java.eval.StarlarkFloat",
        "list" => "net.starlark.java.eval.StarlarkList$MutableList",
        "tuple" => "net.starlark.java.eval.Tuple",
        "dict" => "net.starlark.java.eval.Dict",
        "NoneType" => "net.starlark.java.eval.NoneType",
        "struct" => "com.google.devtools.build.lib.packages.StarlarkInfoNoSchema",
        "Label" => "com.google.devtools.build.lib.cmdline.Label",
        "function" => "net.starlark.java.eval.StarlarkFunction",
        other => return format!("class {}", other),
    };
    format!("class {}", class)
}

/// A Java `Collection` (Starlark list or tuple).
fn as_collection<'v>(v: Value<'v>) -> Option<Vec<Value<'v>>> {
    if let Some(list) = ListRef::from_value(v) {
        return Some(list.content().to_vec());
    }
    TupleRef::from_value(v).map(|t| t.content().to_vec())
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:367 ProjectFunction.isDefaultValue
fn is_default_value(v: Value) -> bool {
    if ListRef::from_value(v).is_some_and(|l| l.is_empty()) {
        return true;
    }
    v.is_none()
}

/// The `Class` of `checkAndCast`.
#[derive(Clone, Copy)]
enum JavaClass {
    Collection,
    String,
    Boolean,
    StarlarkInfoNoSchema,
}

fn is_instance(v: Value, clazz: JavaClass) -> bool {
    match clazz {
        JavaClass::Collection => as_collection(v).is_some(),
        JavaClass::String => v.unpack_str().is_some(),
        JavaClass::Boolean => v.unpack_bool().is_some(),
        JavaClass::StarlarkInfoNoSchema => v.get_type() == "struct",
    }
}

/// Checks that `raw_value` is an instance of `clazz`. If so, returns it. Else if it's an empty
/// `StarlarkList` and `default_value` is not null, returns `None` (the caller uses its default).
/// Else throws a `ProjectFunctionException`.
///
/// Note that all unspecified protolark settings default to an empty `StarlarkList`.
// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:478 ProjectFunction.checkAndCast
fn check_and_cast<'v>(
    raw_value: Option<Value<'v>>,
    clazz: JavaClass,
    has_default_value: bool,
    error_message: &str,
) -> Result<Option<Value<'v>>, ComputeError> {
    let Some(raw_value) = raw_value else {
        // clazz.isInstance(null) is false; rawValue.getClass() throws.
        panic!("NullPointerException: {}", error_message);
    };
    if is_instance(raw_value, clazz) {
        return Ok(Some(raw_value));
    }
    if has_default_value && is_default_value(raw_value) {
        return Ok(None);
    }
    Err(project_function_exception(format!(
        "{}, got {}",
        error_message,
        java_class_of(raw_value)
    )))
}

fn struct_field<'v>(info: Value<'v>, name: &str) -> Option<Value<'v>> {
    blaze_starlark::structs::StarlarkInfo::from_value(info).and_then(|i| i.get_field(name))
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:63 ProjectFunction.compute
fn compute(
    key: &ProjectKey,
    env: &mut Environment<'_>,
) -> Result<Option<ProjectValue>, ComputeError> {
    let bzl_load_value = match env.get_value_or_throw::<BzlLoadKey, BzlLoadFailedException>(
        BzlLoadKey::key_for_build(key.get_project_file().clone()),
    )? {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(None),
        // throw new ProjectFunctionException(e, PERSISTENT)
        Err(e) => return Err(SkyFunctionException::persistent(e).into()),
    };
    let module = bzl_load_value.get_module();
    let project_raw = module
        .get_option(PROJECT_GLOBAL)
        .unwrap_or_else(|e| panic!("{}", e));
    let Some(project_raw) = project_raw else {
        return Err(project_function_exception(
            "Project files must define exactly one top-level variable called \"project\""
                .to_string(),
        ));
    };
    let project_raw = project_raw.value();
    if DictRef::from_value(project_raw).is_some() {
        let actual_project_file = maybe_resolve_alias(key.get_project_file(), project_raw, module)?;
        if actual_project_file != *key.get_project_file() {
            // This is an alias for another project file. Delegate there.
            // TODO: b/382265245 - handle cycles, including self references.
            return match env.get_value_or_throw::<ProjectKey, ProjectFunctionException>(
                ProjectKey::new(actual_project_file),
            )? {
                Ok(v) => Ok(v.map(|v| (*v).clone())),
                Err(e) => Err(SkyFunctionException::persistent(e).into()),
            };
        }
        return parse_legacy_project_schema(project_raw, key.get_project_file()).map(Some);
    }
    if project_raw.get_type() == "struct" {
        return parse_proto_project_schema(project_raw, key.get_project_file()).map(Some);
    }
    Err(project_function_exception(format!(
        "{} variable: expected a map of string to objects, got {}",
        PROJECT_GLOBAL,
        java_class_of(project_raw)
    )))
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:107 ProjectFunction.parseProtoProjectSchema
/// Parses the proto-based PROJECT.scl implementation.
fn parse_proto_project_schema(
    starlark_info: Value,
    project_file: &Label,
) -> Result<ProjectValue, ComputeError> {
    let mut buildable_units_builder: IndexMap<String, BuildableUnit> = IndexMap::new();
    let buildable_units = check_and_cast(
        struct_field(starlark_info, "buildable_units"),
        JavaClass::Collection,
        false,
        "buildable_units must be a list of buildable unit definitions",
    )?
    .expect("no default");
    for raw_buildable_unit in as_collection(buildable_units).expect("a collection") {
        let mut target_patterns_builder: Vec<String> = Vec::new();
        let mut flags_builder: Vec<String> = Vec::new();
        let buildable_unit_struct = check_and_cast(
            Some(raw_buildable_unit),
            JavaClass::StarlarkInfoNoSchema,
            false,
            "buildable_units entries must be structured objects",
        )?
        .expect("no default");
        let buildable_unit_name = check_and_cast(
            struct_field(buildable_unit_struct, "name"),
            JavaClass::String,
            false,
            "buildable_unit names must be strings",
        )?
        .expect("no default")
        .unpack_str()
        .unwrap()
        .to_string();
        let buildable_unit_description = match check_and_cast(
            struct_field(buildable_unit_struct, "description"),
            JavaClass::String,
            true,
            "buildable_unit descriptions must be strings",
        )? {
            Some(v) => v.unpack_str().unwrap().to_string(),
            None => buildable_unit_name.clone(),
        };
        let is_default = match check_and_cast(
            struct_field(buildable_unit_struct, "is_default"),
            JavaClass::Boolean,
            true,
            "is_default must be a boolean",
        )? {
            Some(v) => v.unpack_bool().unwrap(),
            None => false,
        };
        let target_patterns = check_and_cast(
            struct_field(buildable_unit_struct, "target_patterns"),
            JavaClass::Collection,
            true,
            "target_patterns must be a list of strings",
        )?;
        for target_pattern in target_patterns.and_then(as_collection).unwrap_or_default() {
            target_patterns_builder.push(
                check_and_cast(
                    Some(target_pattern),
                    JavaClass::String,
                    false,
                    "target_patterns entries must be strings",
                )?
                .expect("no default")
                .unpack_str()
                .unwrap()
                .to_string(),
            );
        }
        let flags = check_and_cast(
            struct_field(buildable_unit_struct, "flags"),
            JavaClass::Collection,
            true,
            "flags must be a list of strings",
        )?;
        for flag in flags.and_then(as_collection).unwrap_or_default() {
            flags_builder.push(
                check_and_cast(
                    Some(flag),
                    JavaClass::String,
                    false,
                    "flags entries must be strings",
                )?
                .expect("no default")
                .unpack_str()
                .unwrap()
                .to_string(),
            );
        }
        // TODO: b/413130912: cleanly fail when multiple buildable units have the same name.
        let buildable_unit = match BuildableUnit::create(
            buildable_unit_name.clone(),
            target_patterns_builder,
            buildable_unit_description,
            flags_builder,
            is_default,
        ) {
            Ok(b) => b,
            Err(e) => return Err(project_function_exception(e.get_message().to_string())),
        };
        if buildable_units_builder
            .insert(buildable_unit_name.clone(), buildable_unit)
            .is_some()
        {
            return Err(project_function_exception(format!(
                "buildable_unit name='{}' is repeated. Buildable units must have unique names.",
                buildable_unit_name
            )));
        }
    }
    let always_allowed_configs =
        parse_always_allowed_configs(struct_field(starlark_info, "always_allowed_configs"))?;
    Ok(ProjectValue::new(
        parse_enforcement_policy(
            struct_field(starlark_info, ENFORCEMENT_POLICY),
            project_file,
        )?,
        parse_project_directories(struct_field(starlark_info, "project_directories"))?,
        Some(buildable_units_builder),
        if always_allowed_configs.is_empty() {
            None
        } else {
            Some(always_allowed_configs)
        },
        Some(project_file.clone()),
    ))
}

fn dict_get<'v>(dict: Value<'v>, key: &str) -> Option<Value<'v>> {
    let d = DictRef::from_value(dict).expect("a dict");
    d.get_str(key)
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:206 ProjectFunction.parseLegacyProjectSchema
/// Parses the first PROJECT.scl implementation (pre-proto schema).
fn parse_legacy_project_schema(
    dict: Value,
    project_file: &Label,
) -> Result<ProjectValue, ComputeError> {
    let mut buildable_units_builder: IndexMap<String, BuildableUnit> = IndexMap::new();
    {
        let d = DictRef::from_value(dict).expect("a dict");
        for k in d.keys() {
            if k.unpack_str().is_none() {
                return Err(project_function_exception(format!(
                    "{} variable: expected string key, got element of {}",
                    PROJECT_GLOBAL,
                    java_class_of(k)
                )));
            }
        }
    }

    let mut default_config: Option<String> = None;
    if let Some(default_config_raw) = dict_get(dict, "default_config") {
        let default_config_string = check_and_cast(
            Some(default_config_raw),
            JavaClass::String,
            false,
            "default_config must be a string matching a configs variable definition",
        )?
        .expect("no default");
        default_config = Some(default_config_string.unpack_str().unwrap().to_string());
    }
    let mut found_default_config = false;
    let configs_present = dict_get(dict, "configs").is_some();
    if configs_present {
        let configs = parse_configs(dict_get(dict, "configs"), "configs")?;
        for (config, flags) in configs {
            let is_default = default_config.as_deref() == Some(config.as_str());
            if is_default {
                found_default_config = true;
            }
            let buildable_unit = match BuildableUnit::create(
                config.clone(),
                Vec::new(),
                String::new(),
                flags,
                is_default,
            ) {
                Ok(b) => b,
                Err(e) => return Err(project_function_exception(e.get_message().to_string())),
            };
            buildable_units_builder.insert(config, buildable_unit);
        }
    }

    if default_config.is_some() && !found_default_config {
        return Err(project_function_exception(
            "default_config must be a string matching a configs variable definition".to_string(),
        ));
    }

    Ok(ProjectValue::new(
        parse_enforcement_policy(dict_get(dict, ENFORCEMENT_POLICY), project_file)?,
        parse_project_directories(dict_get(dict, "active_directories"))?,
        if configs_present {
            Some(buildable_units_builder)
        } else {
            None
        },
        Some(parse_always_allowed_configs(dict_get(
            dict,
            "always_allowed_configs",
        ))?),
        Some(project_file.clone()),
    ))
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:262 ProjectFunction.parseConfigs
fn parse_configs(
    configs_raw: Option<Value>,
    variable_name: &str,
) -> Result<IndexMap<String, Vec<String>>, ComputeError> {
    // This project file doesn't define configs, so it must not be used for canonical configs.
    let Some(configs_raw) = configs_raw else {
        return Ok(IndexMap::new());
    };
    let mut configs: IndexMap<String, Vec<String>> = IndexMap::new();
    let mut expected_configs_type = false;
    if let Some(configs_as_dict) = DictRef::from_value(configs_raw) {
        expected_configs_type = true;
        for (k, v) in configs_as_dict.iter() {
            let (Some(key), Some(values)) = (k.unpack_str(), as_collection(v)) else {
                expected_configs_type = false;
                break;
            };
            let mut values_builder: Vec<String> = Vec::new();
            for value in values {
                let Some(val) = value.unpack_str() else {
                    expected_configs_type = false;
                    break;
                };
                values_builder.push(val.to_string());
            }
            configs.insert(key.to_string(), values_builder);
        }
    }
    if !expected_configs_type {
        return Err(project_function_exception(format!(
            "{} variable must be a map of strings to lists of strings",
            variable_name
        )));
    }
    Ok(configs)
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:297 ProjectFunction.parseAlwaysAllowedConfigs
fn parse_always_allowed_configs(
    always_allowed_configs_raw: Option<Value>,
) -> Result<Vec<String>, ComputeError> {
    let Some(raw) = always_allowed_configs_raw else {
        return Ok(Vec::new());
    };
    let always_allowed_configs = check_and_cast(
        Some(raw),
        JavaClass::Collection,
        true,
        "always_allowed_configs must be a list of strings",
    )?;
    let mut builder: Vec<String> = Vec::new();
    for config in always_allowed_configs
        .and_then(as_collection)
        .unwrap_or_default()
    {
        builder.push(
            check_and_cast(
                Some(config),
                JavaClass::String,
                false,
                "always_allowed_configs entires must be strings",
            )?
            .expect("no default")
            .unpack_str()
            .unwrap()
            .to_string(),
        );
    }
    Ok(builder)
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:320 ProjectFunction.parseProjectDirectories
fn parse_project_directories(
    active_directories_raw: Option<Value>,
) -> Result<IndexMap<String, Vec<String>>, ComputeError> {
    let active_directories: IndexMap<String, Vec<String>> = match active_directories_raw {
        None => IndexMap::new(),
        Some(raw) if DictRef::from_value(raw).is_some() => {
            let dict = DictRef::from_value(raw).unwrap();
            let mut builder: IndexMap<String, Vec<String>> = IndexMap::new();
            for (k, values) in dict.iter() {
                let Some(active_directories_key) = k.unpack_str() else {
                    return Err(project_function_exception(format!(
                        "expected string, got element of {}",
                        java_class_of(k)
                    )));
                };
                let Some(active_directories_values) = as_collection(values) else {
                    return Err(project_function_exception(format!(
                        "expected list, got element of {}",
                        java_class_of(values)
                    )));
                };
                let mut strings = Vec::new();
                for active_directory in active_directories_values {
                    let Some(s) = active_directory.unpack_str() else {
                        return Err(project_function_exception(format!(
                            "expected a list of strings, got element of {}",
                            java_class_of(active_directory)
                        )));
                    };
                    strings.push(s.to_string());
                }
                // ImmutableMap.Builder.buildOrThrow: Starlark dict keys are unique.
                builder.insert(active_directories_key.to_string(), strings);
            }
            builder
        }
        Some(raw) if ListRef::from_value(raw).is_some() => {
            // The proto schema doesn't need a map. Read a list and store as a {"default": [list}]}
            // map to preserve backward compatibility.
            let mut builder: Vec<String> = Vec::new();
            for active_directory in ListRef::from_value(raw).unwrap().iter() {
                builder.push(
                    check_and_cast(
                        Some(active_directory),
                        JavaClass::String,
                        false,
                        "project_directories is a list of strings",
                    )?
                    .expect("no default")
                    .unpack_str()
                    .unwrap()
                    .to_string(),
                );
            }
            IndexMap::from([("default".to_string(), builder)])
        }
        Some(raw) => {
            return Err(project_function_exception(format!(
                "expected a map of string to list of strings, got {}",
                java_class_of(raw)
            )));
        }
    };
    if !active_directories.is_empty() && !active_directories.contains_key("default") {
        return Err(project_function_exception(
            "non-empty active_directories must contain the 'default' key".to_string(),
        ));
    }
    Ok(active_directories)
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:377 ProjectFunction.parseEnforcementPolicy
fn parse_enforcement_policy(
    enforcement_policy_raw: Option<Value>,
    project_file: &Label,
) -> Result<EnforcementPolicy, ComputeError> {
    let Some(raw) = enforcement_policy_raw.filter(|v| !is_default_value(*v)) else {
        // Default if unspecified.
        return Ok(EnforcementPolicy::WARN);
    };
    // enforcementPolicyRaw.toString(): a Starlark string's toString is its content.
    let text = match raw.unpack_str() {
        Some(s) => s.to_string(),
        None => raw.to_str(),
    };
    // toLowerCase(Locale.ROOT)
    EnforcementPolicy::from_string(&text.to_lowercase())
        .map_err(|e| project_function_exception(format!("{} in {}", e, project_file)))
}

// Java: com/google/devtools/build/lib/skyframe/ProjectFunction.java:400 ProjectFunction.maybeResolveAlias
/// If this is an alias for another project file, returns its label. Else returns the original
/// key's label.
fn maybe_resolve_alias(
    original_project_file: &Label,
    project: Value,
    module: &starlark::environment::FrozenModule,
) -> Result<Label, ComputeError> {
    let d = DictRef::from_value(project).expect("a dict");
    let Some(actual) = d.get_str("actual") else {
        return Ok(original_project_file.clone());
    };
    let Some(actual_str) = actual.unpack_str() else {
        return Err(project_function_exception(format!(
            "project[\"actual\"]: expected string, got {}",
            actual.to_str()
        )));
    };
    if d.len() > 1 {
        return Err(project_function_exception(format!(
            "project[\"actual\"] is present, but other keys are present as well: [{}]",
            d.keys().map(|k| k.to_str()).collect::<Vec<_>>().join(", ")
        )));
    }
    let globals: Vec<String> = module
        .names_any_visibility()
        .map(|n| n.as_str().to_string())
        .collect();
    if globals.len() > 1 {
        return Err(project_function_exception(format!(
            "project global variable is present, but other globals are present as well: [{}]",
            globals.join(", ")
        )));
    }
    Label::parse_canonical(actual_str)
        .map_err(|e| project_function_exception(e.get_message().to_string()))
}

/// Registers `PROJECT_FILES_LOOKUP` and `PROJECT`.
// Java: com/google/devtools/build/lib/skyframe/SkyframeExecutor.java:760 SkyframeExecutor.skyFunctions
pub fn register(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .typed_function::<ProjectFilesLookupKey, _>(project_files_lookup_function)
        .typed_function::<ProjectKey, _>(compute)
}
