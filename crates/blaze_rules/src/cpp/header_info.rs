//! `CcCompilationContext.HeaderInfo`: the header-module data of one C++ compilation context and
//! its dependency structure, as created by `cc_internal.create_header_info(...)` and
//! `cc_internal.create_header_info_with_deps(...)`.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::StarlarkFile;
use blaze_starlark::structs::DeeplyImmutable;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::collections::StarlarkHasher;
use starlark::starlark_simple_value;
use starlark::values::ValueLike;
use starlark::values::list::alloc_immutable_list;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use super::cpp_file_types;

/// The data of a `HeaderInfo`. Identity (Java's `identityToken`) is the `Arc`'s address.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:599 CcCompilationContext.HeaderInfo
#[derive(Debug)]
pub struct HeaderInfoData {
    /// The modules built for this context. If null, then no module is being compiled for this
    /// context.
    pub header_module: Option<Artifact>,
    pub pic_header_module: Option<Artifact>,
    /// All public header files that are compiled into this module.
    pub modular_public_headers: Vec<Artifact>,
    /// All private header files that are compiled into this module.
    pub modular_private_headers: Vec<Artifact>,
    /// All textual header files that are contained in this module.
    pub textual_headers: Vec<Artifact>,
    /// Headers that can be compiled into a separate, smaller module for performance reasons.
    pub separate_module_headers: Vec<Artifact>,
    pub separate_module: Option<Artifact>,
    pub separate_pic_module: Option<Artifact>,
    /// HeaderInfos of direct dependencies of C++ target represented by this context.
    pub deps: Vec<Arc<HeaderInfoData>>,
}

impl HeaderInfoData {
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:672 HeaderInfo.getModule
    pub fn get_module(&self, pic: bool) -> Option<&Artifact> {
        if pic {
            self.pic_header_module.as_ref()
        } else {
            self.header_module.as_ref()
        }
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:676 HeaderInfo.getSeparateModule
    pub fn get_separate_module(&self, pic: bool) -> Option<&Artifact> {
        if pic {
            self.separate_pic_module.as_ref()
        } else {
            self.separate_module.as_ref()
        }
    }

    /// Java `HeaderInfo.EMPTY` (a fresh identity each call; Java's EMPTY uses the constant
    /// symbol, and no code path compares two empty infos by identity).
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:630 HeaderInfo.EMPTY
    pub fn empty() -> Arc<HeaderInfoData> {
        Arc::new(HeaderInfoData {
            header_module: None,
            pic_header_module: None,
            modular_public_headers: Vec::new(),
            modular_private_headers: Vec::new(),
            textual_headers: Vec::new(),
            separate_module_headers: Vec::new(),
            separate_module: None,
            separate_pic_module: None,
            deps: Vec::new(),
        })
    }

    /// Creates a new HeaderInfo instance.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:869 HeaderInfo.create
    pub fn create(
        header_module: Option<Artifact>,
        pic_header_module: Option<Artifact>,
        public_headers: &[Artifact],
        private_headers: &[Artifact],
        textual_headers: &[Artifact],
        separate_module_headers: Vec<Artifact>,
        separate_module: Option<Artifact>,
        separate_pic_module: Option<Artifact>,
        deps: Vec<Arc<HeaderInfoData>>,
        merged_deps: &[Arc<HeaderInfoData>],
    ) -> Arc<HeaderInfoData> {
        blaze_util::check_state!(
            (separate_module.is_none() || header_module.is_some())
                && (separate_pic_module.is_none() || pic_header_module.is_some()),
            "Separate module ('{}', '{}') cannot be used without main module",
            separate_module
                .as_ref()
                .map_or("null".to_owned(), |a| a.to_string()),
            separate_pic_module
                .as_ref()
                .map_or("null".to_owned(), |a| a.to_string())
        );
        let mut modular_public: IndexSet<Artifact> = IndexSet::new();
        let mut modular_private: IndexSet<Artifact> = IndexSet::new();
        let mut all_textual: IndexSet<Artifact> = IndexSet::new();
        all_textual.extend(textual_headers.iter().cloned());
        // TODO(djasper): CPP_TEXTUAL_INCLUDEs are currently special cased here and in
        // CppModuleMapAction. These should be moved to a place earlier in the Action
        // construction.
        for header in public_headers {
            if cpp_file_types::CPP_TEXTUAL_INCLUDE.matches_artifact(header) {
                all_textual.insert(header.clone());
            } else {
                modular_public.insert(header.clone());
            }
        }
        for header in private_headers {
            if cpp_file_types::CPP_TEXTUAL_INCLUDE.matches_artifact(header) {
                all_textual.insert(header.clone());
            } else {
                modular_private.insert(header.clone());
            }
        }
        for other in merged_deps {
            modular_public.extend(other.modular_public_headers.iter().cloned());
            modular_private.extend(other.modular_private_headers.iter().cloned());
            all_textual.extend(other.textual_headers.iter().cloned());
        }
        Arc::new(HeaderInfoData {
            header_module,
            pic_header_module,
            modular_public_headers: modular_public.into_iter().collect(),
            modular_private_headers: modular_private.into_iter().collect(),
            textual_headers: all_textual.into_iter().collect(),
            separate_module_headers,
            separate_module,
            separate_pic_module,
            deps,
        })
    }

    /// Java `getTransitiveCollection()`: this info and every transitive dependency, each once
    /// (by identity), in Java's iteration order: the largest dependency's collection is reused
    /// and the others are appended depth-first.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:738 HeaderInfo.getTransitiveCollection
    pub fn get_transitive_collection(self: &Arc<HeaderInfoData>) -> Vec<Arc<HeaderInfoData>> {
        if self.deps.is_empty() {
            return vec![self.clone()];
        }
        // Java: flatten() + TransitiveHeaderIterator. The memoized structure is (largest dep,
        // additional deps); iteration yields `this`, the additional deps, then continues with
        // the largest dep's own iteration.
        let mut largest: Option<(&Arc<HeaderInfoData>, Vec<Arc<HeaderInfoData>>)> = None;
        for dep in &self.deps {
            let dep_list = dep.get_transitive_collection();
            let larger = match &largest {
                None => !dep_list.is_empty(),
                Some((_, l)) => dep_list.len() > l.len(),
            };
            if larger {
                largest = Some((dep, dep_list));
            }
        }
        let (largest_dep, largest_list) = match largest {
            Some((d, l)) => (Some(d.clone()), l),
            None => (None, Vec::new()),
        };
        let mut result: HashSet<usize> = largest_list.iter().map(identity).collect();
        result.insert(identity(self));
        let mut additional_deps = Vec::new();
        for dep in &self.deps {
            add_others(dep, &mut result, &mut additional_deps);
        }
        let mut out = vec![self.clone()];
        out.extend(additional_deps);
        if let Some(largest_dep) = largest_dep {
            out.extend(largest_dep.get_transitive_collection());
        }
        out
    }
}

fn identity(h: &Arc<HeaderInfoData>) -> usize {
    Arc::as_ptr(h) as usize
}

// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:775 HeaderInfo.addOthers
fn add_others(
    this: &Arc<HeaderInfoData>,
    result: &mut HashSet<usize>,
    additional_deps: &mut Vec<Arc<HeaderInfoData>>,
) {
    if result.insert(identity(this)) {
        additional_deps.push(this.clone());
        for dep in &this.deps {
            add_others(dep, result, additional_deps);
        }
    }
}

/// The Starlark value of a `HeaderInfo`.
// Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:599 CcCompilationContext.HeaderInfo
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct HeaderInfo(#[allocative(skip)] pub Arc<HeaderInfoData>);

starlark_simple_value!(HeaderInfo);

impl fmt::Debug for HeaderInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HeaderInfo({:p})", Arc::as_ptr(&self.0))
    }
}

impl fmt::Display for HeaderInfo {
    // Java: StarlarkValue.repr default.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "<unknown object com.google.devtools.build.lib.rules.cpp.CcCompilationContext$HeaderInfo>",
        )
    }
}

fn file_or_none<'v>(a: &Option<Artifact>, heap: Heap<'v>) -> Value<'v> {
    match a {
        Some(a) => heap.alloc(StarlarkFile::new(a.clone())),
        None => Value::new_none(),
    }
}

/// `StarlarkList.immutableCopyOf(artifacts)`.
// Java: net/starlark/java/eval/StarlarkList.java:168 StarlarkList.immutableCopyOf
fn file_list<'v>(artifacts: &[Artifact], heap: Heap<'v>) -> Value<'v> {
    let files: Vec<Value<'v>> = artifacts
        .iter()
        .map(|a| heap.alloc(StarlarkFile::new(a.clone())))
        .collect();
    alloc_immutable_list(heap, &files)
}

#[starlark_value(type = "HeaderInfo")]
impl<'v> StarlarkValue<'v> for HeaderInfo {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        let d = &self.0;
        match attribute {
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:684 HeaderInfo.getSeparateModule
            "separate_module" => Some(file_or_none(&d.separate_module, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:694 HeaderInfo.getHeaderModule
            "header_module" => Some(file_or_none(&d.header_module, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:704 HeaderInfo.getPicHeaderModule
            "pic_header_module" => Some(file_or_none(&d.pic_header_module, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:709 HeaderInfo.getModularPublicHeaders
            "modular_public_headers" => Some(file_list(&d.modular_public_headers, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:714 HeaderInfo.getModularPrivateHeaders
            "modular_private_headers" => Some(file_list(&d.modular_private_headers, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:719 HeaderInfo.getTextualHeaders
            "textual_headers" => Some(file_list(&d.textual_headers, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:724 HeaderInfo.getSeparateModuleHeaders
            "separate_module_headers" => Some(file_list(&d.separate_module_headers, heap)),
            // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:734 HeaderInfo.getSeparatePicModule
            "separate_pic_module" => Some(file_or_none(&d.separate_pic_module, heap)),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        self.get_attr(attribute, heap).is_some()
    }

    fn dir_attr(&self) -> Vec<String> {
        [
            "header_module",
            "modular_private_headers",
            "modular_public_headers",
            "pic_header_module",
            "separate_module",
            "separate_module_headers",
            "separate_pic_module",
            "textual_headers",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:785 HeaderInfo.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(header_info_of(other).is_some_and(|o| Arc::ptr_eq(&o, &self.0)))
    }

    /// Hashable (Java `isImmutable()` is true, `StarlarkValue.checkHashable`) by identity: the
    /// `Arc`'s address stands for Java's `identityToken`.
    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:793 HeaderInfo.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        (Arc::as_ptr(&self.0) as usize).hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcCompilationContext.java:798 HeaderInfo.isImmutable
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// The `HeaderInfo` data behind `x`: this crate's value, or a `HeaderInfo` the loading-phase
/// `cc_internal` (blaze_packages) created while a .bzl file was initialized.
///
/// PORT: a loading-phase HeaderInfo has no artifacts (none exist while loading) and is created
/// by `create_header_info()` with no dependencies (rules_cc's `cc_info.bzl` top-level empty
/// compilation context); its data is the empty HeaderInfo, one per loading-phase value.
pub fn header_info_of(x: Value) -> Option<Arc<HeaderInfoData>> {
    if let Some(h) = x.downcast_ref::<HeaderInfo>() {
        return Some(h.0.clone());
    }
    if let Some(h) = x.downcast_ref::<blaze_packages::builtins::cc_internal::HeaderInfo>() {
        return Some(loading_phase_empty_header_info(&h.0));
    }
    None
}

/// The empty HeaderInfo standing for a loading-phase HeaderInfo, one per loading-phase value
/// (the memo keeps the loading-phase data alive so its address is not reused).
fn loading_phase_empty_header_info(
    loading: &Arc<blaze_packages::builtins::cc_internal::HeaderInfoData>,
) -> Arc<HeaderInfoData> {
    use std::collections::HashMap;
    use std::sync::LazyLock;
    type Memo = HashMap<
        usize,
        (
            Arc<blaze_packages::builtins::cc_internal::HeaderInfoData>,
            Arc<HeaderInfoData>,
        ),
    >;
    static MEMO: LazyLock<parking_lot::Mutex<Memo>> =
        LazyLock::new(|| parking_lot::Mutex::new(HashMap::new()));
    let key = Arc::as_ptr(loading) as usize;
    MEMO.lock()
        .entry(key)
        .or_insert_with(|| (loading.clone(), HeaderInfoData::empty()))
        .1
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(deps: Vec<Arc<HeaderInfoData>>) -> Arc<HeaderInfoData> {
        HeaderInfoData::create(None, None, &[], &[], &[], Vec::new(), None, None, deps, &[])
    }

    /// Java: `HeaderInfo.isImmutable()` is true and `equals`/`hashCode` use the identity token
    /// (CcCompilationContext.java:785-800), so a HeaderInfo is a valid dict key and depset
    /// element, two values of one HeaderInfo are the same key, and two HeaderInfos with equal
    /// contents are different keys (analysis review S1-7; end to end:
    /// scripts/w4/scenarios/cc-hash).
    #[test]
    fn header_info_is_hashable_by_identity() {
        use starlark::values::Heap;
        Heap::temp(|heap| {
            let a = info(Vec::new());
            let b = info(Vec::new());
            let a1 = heap.alloc(HeaderInfo(a.clone()));
            let a2 = heap.alloc(HeaderInfo(a.clone()));
            let b1 = heap.alloc(HeaderInfo(b.clone()));
            let ha1 = a1.get_hashed().expect("HeaderInfo is hashable");
            let ha2 = a2.get_hashed().expect("HeaderInfo is hashable");
            b1.get_hashed().expect("HeaderInfo is hashable");
            assert_eq!(ha1.hash(), ha2.hash());
            assert!(a1.equals(a2).unwrap());
            assert!(!a1.equals(b1).unwrap());
        });
    }

    #[test]
    fn transitive_collection_without_deps_is_self() {
        let a = info(Vec::new());
        let c = a.get_transitive_collection();
        assert_eq!(c.len(), 1);
        assert!(Arc::ptr_eq(&c[0], &a));
    }

    #[test]
    fn transitive_collection_order_matches_java_iterator() {
        // A -> [B, C], B -> [D], C -> [D]. Java's TransitiveHeaderIterator yields A, the
        // additional deps of A ([C]), then the largest dep's collection (B, D).
        let d = info(Vec::new());
        let b = info(vec![d.clone()]);
        let c = info(vec![d.clone()]);
        let a = info(vec![b.clone(), c.clone()]);
        let order: Vec<usize> = a
            .get_transitive_collection()
            .iter()
            .map(|h| Arc::as_ptr(h) as usize)
            .collect();
        let expected: Vec<usize> = [&a, &c, &b, &d]
            .iter()
            .map(|h| Arc::as_ptr(h) as usize)
            .collect();
        assert_eq!(order, expected);
    }
}
