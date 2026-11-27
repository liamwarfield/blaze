//! `CcCompilationContext`: the Java view over rules_cc's `CcCompilationContextInfo` provider
//! instance (`CcCompilationContext.of(starlarkInfo)`).
//!
//! PORT: Java keeps the `StarlarkInfo` and reads fields on demand. The compile action outlives
//! the target's Starlark heap, so the fields the action reads are read once, when the action is
//! built ([`CcCompilationContext::of`]); the values are immutable, so this is equivalent. Fields
//! only read during execution (include directories) are read leniently and fail when used.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::StarlarkInfo;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;
use starlark::values::Value;

use super::header_info::{HeaderInfoData, header_info_of};
use super::include_scanner::IncludeScanningHeaderDataBuilder;
use super::util::{depset_get_set_artifacts, depset_get_set_strings, field_type_error};

/// Immutable data of one compilation context, as the compile action uses it.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:62 CcCompilationContext
#[derive(Debug, Clone)]
pub struct CcCompilationContext {
    declared_include_srcs: NestedSet<Artifact>,
    transitive_modules: NestedSet<Artifact>,
    transitive_pic_modules: NestedSet<Artifact>,
    direct_module_maps: NestedSet<Artifact>,
    non_code_inputs: NestedSet<Artifact>,
    cpp_module_map: Option<Artifact>,
    header_info: Arc<HeaderInfoData>,
    include_dirs: Result<Vec<PathFragment>, String>,
    quote_include_dirs: Result<Vec<PathFragment>, String>,
    system_include_dirs: Result<Vec<PathFragment>, String>,
    framework_include_dirs: Result<Vec<PathFragment>, String>,
    external_include_dirs: Result<Vec<PathFragment>, String>,
}

/// Java `starlarkInfo.getValue(field, Depset.class).getSet(Artifact.class)` wrapped in
/// `IllegalStateException` (as every `CcCompilationContext` getter does).
fn artifact_set(info: &StarlarkInfo, field: &str) -> NestedSet<Artifact> {
    match artifact_set_checked(info, field) {
        Ok(s) => s,
        Err(e) => panic!("IllegalStateException: {e}"),
    }
}

fn artifact_set_checked(info: &StarlarkInfo, field: &str) -> Result<NestedSet<Artifact>, String> {
    let Some(v) = info.get_field(field) else {
        // Java: getValue returns null, then `.getSet` throws NullPointerException.
        panic!("NullPointerException: CcCompilationContext field '{field}'");
    };
    if Depset::from_value(v).is_none() {
        return Err(field_type_error(field, v, "depset").to_string());
    }
    depset_get_set_artifacts(v)
}

/// Java `getPathFragmentList(fieldName)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:82 CcCompilationContext.getPathFragmentList
fn path_fragment_list(info: &StarlarkInfo, field: &str) -> Result<Vec<PathFragment>, String> {
    let Some(v) = info.get_field(field) else {
        return Err(format!("NullPointerException: field '{field}'"));
    };
    if Depset::from_value(v).is_none() {
        return Err(field_type_error(field, v, "depset").to_string());
    }
    Ok(depset_get_set_strings(v)?
        .to_list()
        .iter()
        .map(|s| PathFragment::create(s))
        .collect())
}

impl CcCompilationContext {
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:70 CcCompilationContext.of
    pub fn of(info: &StarlarkInfo) -> CcCompilationContext {
        // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:210 CcCompilationContext.getHeaderInfo
        let header_info = match info.get_field("_header_info") {
            Some(v) => match header_info_of(v) {
                Some(h) => h,
                None => panic!(
                    "IllegalStateException: {}",
                    field_type_error("_header_info", v, "HeaderInfo")
                ),
            },
            None => panic!("NullPointerException: CcCompilationContext field '_header_info'"),
        };
        CcCompilationContext {
            declared_include_srcs: artifact_set(info, "headers"),
            transitive_modules: artifact_set(info, "_transitive_modules"),
            transitive_pic_modules: artifact_set(info, "_transitive_pic_modules"),
            direct_module_maps: artifact_set(info, "_direct_module_maps"),
            non_code_inputs: artifact_set(info, "_non_code_inputs"),
            cpp_module_map: get_cpp_module_map(info),
            header_info,
            include_dirs: path_fragment_list(info, "includes"),
            quote_include_dirs: path_fragment_list(info, "quote_includes"),
            system_include_dirs: path_fragment_list(info, "system_includes"),
            framework_include_dirs: path_fragment_list(info, "framework_includes"),
            external_include_dirs: path_fragment_list(info, "external_includes"),
        }
    }

    /// Java `CcCompilationContext.of((StarlarkInfo) value)` for a Starlark value.
    pub fn of_value(v: Value) -> starlark::Result<CcCompilationContext> {
        let Some(info) = StarlarkInfo::from_value(v) else {
            return Err(super::util::errorf(format!(
                "in call to create_cc_compile_action(), parameter 'cc_compilation_context' got \
                 value of type '{}', want 'struct'",
                v.get_type()
            )));
        };
        Ok(CcCompilationContext::of(info))
    }

    /// Returns the immutable set of headers that have been declared in the `srcs` or `hdrs`
    /// attribute (possibly empty but never null).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:155 CcCompilationContext.getDeclaredIncludeSrcs
    pub fn get_declared_include_srcs(&self) -> &NestedSet<Artifact> {
        &self.declared_include_srcs
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:446 CcCompilationContext.getTransitiveModules
    pub fn get_transitive_modules(&self, use_pic: bool) -> &NestedSet<Artifact> {
        if use_pic {
            &self.transitive_pic_modules
        } else {
            &self.transitive_modules
        }
    }

    /// Adds additional transitive inputs needed for compilation to builder.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:477 CcCompilationContext.addAdditionalInputs
    pub fn add_additional_inputs(&self, builder: &mut NestedSetBuilder<Artifact>) {
        builder.add_transitive(&self.direct_module_maps);
        builder.add_transitive(&self.non_code_inputs);
        if let Some(module_map) = &self.cpp_module_map {
            builder.add(module_map.clone());
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:209 CcCompilationContext.getHeaderInfo
    pub fn get_header_info(&self) -> &Arc<HeaderInfoData> {
        &self.header_info
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:492 CcCompilationContext.getHeaderModule
    pub fn get_header_module(&self, use_pic: bool) -> Option<&Artifact> {
        self.header_info.get_module(use_pic)
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:496 CcCompilationContext.getSeparateHeaderModule
    pub fn get_separate_header_module(&self, use_pic: bool) -> Option<&Artifact> {
        self.header_info.get_separate_module(use_pic)
    }

    /// Returns the modules that a compilation in this context could use directly: the modules
    /// of its direct dependencies as well as the separate module of this context (except for
    /// the compile of the separate module itself, which is indicated by `separate`).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:408 CcCompilationContext.getDirectModules
    pub fn get_direct_modules(&self, use_pic: bool, separate: bool) -> NestedSet<Artifact> {
        let header_info = &self.header_info;
        let mut modules: IndexSet<Artifact> = IndexSet::new();
        for dep in &header_info.deps {
            collect_modules(dep, use_pic, &mut modules);
        }
        // The separate module can be used by all compiles of this context except for its own
        // compile.
        if let Some(separate_module) = header_info.get_separate_module(use_pic) {
            if !separate {
                modules.insert(separate_module.clone());
            }
        }
        NestedSetBuilder::wrap(Order::STABLE_ORDER, modules)
    }

    /// Returns the immutable list of include directories to be added with "-I" (possibly empty
    /// but never null).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:99 CcCompilationContext.getIncludeDirs
    pub fn get_include_dirs(&self) -> Vec<PathFragment> {
        self.include_dirs
            .clone()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    /// Returns the immutable list of include directories to be added with "-F" (possibly empty
    /// but never null).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:132 CcCompilationContext.getFrameworkIncludeDirs
    pub fn get_framework_include_dirs(&self) -> Vec<PathFragment> {
        self.framework_include_dirs
            .clone()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:110 CcCompilationContext.getQuoteIncludeDirs
    pub fn get_quote_include_dirs(&self) -> Vec<PathFragment> {
        self.quote_include_dirs
            .clone()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:121 CcCompilationContext.getSystemIncludeDirs
    pub fn get_system_include_dirs(&self) -> Vec<PathFragment> {
        self.system_include_dirs
            .clone()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:141 CcCompilationContext.getExternalIncludeDirs
    pub fn get_external_include_dirs(&self) -> Vec<PathFragment> {
        self.external_include_dirs
            .clone()
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    /// Returns `None` when a required SkyValue is missing and a Skyframe restart is required.
    ///
    /// `env` is `actionExecutionContext.getEnvironmentForDiscoveringInputs()`, as Java passes it.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:300 CcCompilationContext.createIncludeScanningHeaderData
    pub fn create_include_scanning_header_data(
        &self,
        env: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
        use_pic: bool,
        create_modular_headers: bool,
    ) -> Option<IncludeScanningHeaderDataBuilder> {
        let header_info = self.get_header_info();
        let transitive_header_infos = header_info.get_transitive_collection();
        let mut tree_artifacts: Vec<Artifact> = Vec::new();
        let mut path_to_legal_artifact: HashMap<PathFragment, Artifact> =
            HashMap::with_capacity(transitive_header_infos.len());
        let mut modular_headers: HashSet<Artifact> =
            HashSet::with_capacity(transitive_header_infos.len());
        for transitive_header_info in &transitive_header_infos {
            let is_module =
                create_modular_headers && transitive_header_info.get_module(use_pic).is_some();
            handle_headers_for_include_scanning(
                &transitive_header_info.modular_public_headers,
                &mut path_to_legal_artifact,
                &mut tree_artifacts,
                is_module,
                Some(&mut modular_headers),
            );
            handle_headers_for_include_scanning(
                &transitive_header_info.modular_private_headers,
                &mut path_to_legal_artifact,
                &mut tree_artifacts,
                is_module,
                Some(&mut modular_headers),
            );
            handle_headers_for_include_scanning(
                &transitive_header_info.separate_module_headers,
                &mut path_to_legal_artifact,
                &mut tree_artifacts,
                is_module,
                Some(&mut modular_headers),
            );
            handle_headers_for_include_scanning(
                &transitive_header_info.textual_headers,
                &mut path_to_legal_artifact,
                &mut tree_artifacts,
                /* is_module= */ false,
                None,
            );
        }
        if !handle_tree_artifacts(env, &mut path_to_legal_artifact, &tree_artifacts) {
            return None;
        }
        remove_artifacts_from_set(&mut modular_headers, &header_info.modular_public_headers);
        remove_artifacts_from_set(&mut modular_headers, &header_info.modular_private_headers);
        remove_artifacts_from_set(&mut modular_headers, &header_info.textual_headers);
        remove_artifacts_from_set(&mut modular_headers, &header_info.separate_module_headers);
        Some(IncludeScanningHeaderDataBuilder::new(
            path_to_legal_artifact,
            modular_headers,
        ))
    }

    /// Returns a list of all headers from `includes` that are properly declared as well as all
    /// the modules that they are in (in the order of the transitive header infos).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:355 CcCompilationContext.computeUsedModules
    pub fn compute_used_modules(
        &self,
        use_pic: bool,
        includes: &HashSet<Artifact>,
        separate: bool,
    ) -> Vec<Artifact> {
        let header_info = self.get_header_info();
        let mut modules: IndexSet<Artifact> = IndexSet::new();
        for transitive_header_info in header_info.get_transitive_collection() {
            let Some(module) = transitive_header_info.get_module(use_pic) else {
                // If we don't have a main module, there is also not going to be a separate
                // module. This is verified when constructing HeaderInfo instances.
                continue;
            };
            if transitive_header_info
                .modular_public_headers
                .iter()
                .any(|h| includes.contains(h))
            {
                modules.insert(module.clone());
            }
            if transitive_header_info
                .modular_private_headers
                .iter()
                .any(|h| includes.contains(h))
            {
                modules.insert(module.clone());
            }
            if transitive_header_info
                .separate_module_headers
                .iter()
                .any(|h| includes.contains(h))
            {
                modules.insert(
                    transitive_header_info
                        .get_separate_module(use_pic)
                        .unwrap_or_else(|| panic!("NullPointerException: separate module"))
                        .clone(),
                );
            }
        }
        // Do not add the module of the current rule for both:
        // 1. the module compile itself
        // 2. compiles of other translation units of the same rule.
        let own = if separate {
            header_info.get_separate_module(use_pic)
        } else {
            header_info.get_module(use_pic)
        };
        if let Some(own) = own {
            modules.shift_remove(own);
        }
        modules.into_iter().collect()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:506 CcCompilationContext.getHeaderModuleSrcs
    pub fn get_header_module_srcs(&self, separate_module: bool) -> Vec<Artifact> {
        let header_info = self.get_header_info();
        if separate_module {
            return header_info.separate_module_headers.clone();
        }
        let mut set: IndexSet<Artifact> = IndexSet::new();
        set.extend(header_info.modular_public_headers.iter().cloned());
        set.extend(header_info.modular_private_headers.iter().cloned());
        set.extend(header_info.textual_headers.iter().cloned());
        set.extend(header_info.separate_module_headers.iter().cloned());
        set.into_iter().collect()
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:222 IncludeScanningHeaderDataHelper.handleArtifact
fn handle_artifact(
    artifact: &Artifact,
    path_to_legal_artifact: &mut HashMap<PathFragment, Artifact>,
    tree_artifacts: &mut Vec<Artifact>,
) {
    if artifact.is_tree_artifact() {
        tree_artifacts.push(artifact.clone());
        return;
    }
    path_to_legal_artifact.insert(artifact.get_exec_path().clone(), artifact.clone());
}

/// Enter the TreeArtifactValues in each TreeArtifact into pathToLegalArtifact. Returns true on
/// success.
///
/// If a TreeArtifact's value is missing, returns false, and leave pathToLegalArtifact
/// unmodified.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:240 IncludeScanningHeaderDataHelper.handleTreeArtifacts
pub fn handle_tree_artifacts(
    env: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
    path_to_legal_artifact: &mut HashMap<PathFragment, Artifact>,
    tree_artifacts: &[Artifact],
) -> bool {
    if tree_artifacts.is_empty() {
        return true;
    }
    // env.getValuesAndExceptions(treeArtifacts); if (env.valuesMissing()) return false. A value
    // missing although none was reported missing is Java's BugReport and `return false`, which
    // the all-or-nothing read folds into the same answer.
    let mut values = None;
    env.with_environment_for_discovering_inputs(&mut |env| {
        values = env.get_tree_artifact_values(tree_artifacts);
    });
    let Some(values) = values else {
        return false;
    };
    for tree_artifact_value in values {
        for tree_file_artifact in tree_artifact_value.get_children() {
            let tree_file_artifact =
                blaze_exec::actions::action_adapters::as_blaze_artifact(&tree_file_artifact)
                    .unwrap_or_else(|| {
                        panic!(
                            "ClassCastException: {tree_file_artifact:?} is not a TreeFileArtifact"
                        )
                    })
                    .clone();
            path_to_legal_artifact.insert(
                tree_file_artifact.get_exec_path().clone(),
                tree_file_artifact,
            );
        }
    }
    true
}

/// Passes a list of headers to the include scanning helper for handling, and optionally adds
/// them to a set that tracks modular headers.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:278 CcCompilationContext.handleHeadersForIncludeScanning
fn handle_headers_for_include_scanning(
    headers: &[Artifact],
    path_to_legal_artifact: &mut HashMap<PathFragment, Artifact>,
    tree_artifacts: &mut Vec<Artifact>,
    is_module: bool,
    mut modular_headers: Option<&mut HashSet<Artifact>>,
) {
    for a in headers {
        handle_artifact(a, path_to_legal_artifact, tree_artifacts);
        if is_module {
            modular_headers
                .as_deref_mut()
                .unwrap_or_else(|| panic!("NullPointerException: modularHeaders"))
                .insert(a.clone());
        }
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:437 CcCompilationContext.removeArtifactsFromSet
fn remove_artifacts_from_set(set: &mut HashSet<Artifact>, artifacts: &[Artifact]) {
    // Not using iterators here as the resulting overhead is significant in profiles. Do not use
    // Iterables.removeAll() or Set.removeAll() here as with the given container sizes, that
    // needlessly deteriorates to a quadratic algorithm.
    for artifact in artifacts {
        set.remove(artifact);
    }
}

// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:423 CcCompilationContext.collectModules
fn collect_modules(header_info: &HeaderInfoData, use_pic: bool, modules: &mut IndexSet<Artifact>) {
    let Some(module) = header_info.get_module(use_pic) else {
        // If we don't have a main module, there is also not going to be a separate module. This
        // is verified when constructing HeaderInfo instances.
        return;
    };
    modules.insert(module.clone());
    if let Some(separate_module) = header_info.get_separate_module(use_pic) {
        modules.insert(separate_module.clone());
    }
}

/// Returns the C++ module map of the owner (its `file`).
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:546 CcCompilationContext.getCppModuleMap
// Java: com/google/devtools/build/lib/rules/cpp/CppModuleMap.java:30 CppModuleMap.getArtifact
// Java: com/google/devtools/build/lib/rules/cpp/CppModuleMap.java:26 CppModuleMap.<init>
// PORT: skipped, no caller at the pin (the module map's name is read by the Starlark rules, not by a native): CppModuleMap.getName
fn get_cpp_module_map(info: &StarlarkInfo) -> Option<Artifact> {
    let v = info.get_field("_module_map")?;
    if v.is_none() {
        return None;
    }
    let Some(module_map) = StarlarkInfo::from_value(v) else {
        panic!(
            "IllegalStateException: {}",
            field_type_error("_module_map", v, "struct")
        );
    };
    match module_map.get_field("file") {
        Some(f) => match blaze_actions::analysis::starlark::starlark_file::artifact_from_value(f) {
            Some(a) => Some(a),
            None => panic!(
                "IllegalStateException: {}",
                field_type_error("file", f, "File")
            ),
        },
        None => panic!("NullPointerException: CppModuleMap.getArtifact"),
    }
}
