//! `CcCompilationContext`: the Java view over rules_cc's `CcCompilationContextInfo` provider
//! instance (`CcCompilationContext.of(starlarkInfo)`).
//!
//! PORT: Java keeps the `StarlarkInfo` and reads fields on demand. The compile action outlives
//! the target's Starlark heap, so the fields the action reads are read once, when the action is
//! built ([`CcCompilationContext::of`]); the values are immutable, so this is equivalent. Fields
//! only read during execution (include directories) are read leniently and fail when used.

use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::StarlarkInfo;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;
use starlark::values::Value;

use super::header_info::{HeaderInfoData, header_info_of};
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
    quote_include_dirs: Result<Vec<PathFragment>, String>,
    system_include_dirs: Result<Vec<PathFragment>, String>,
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
            quote_include_dirs: path_fragment_list(info, "quote_includes"),
            system_include_dirs: path_fragment_list(info, "system_includes"),
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
