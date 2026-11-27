//! Port of `starlarkdocextract/ModuleInfoExtractor.java`: structured documentation for the
//! loadable symbols of a compiled, loaded Starlark module.

use std::collections::HashMap;

use blaze_packages::aspect::aspect_propagation_edges_supplier::AspectPropagationEdgesSupplier;
use blaze_packages::rule_class::provider::ProviderKey;
use blaze_packages::starlark_defs::aspect::aspect_data;
use blaze_packages::starlark_defs::documentable::{
    documentable_module_extension, documentable_repo_rule,
};
use blaze_packages::starlark_defs::macro_function::macro_function_state;
use blaze_packages::starlark_defs::provider::StarlarkProvider;
use blaze_packages::starlark_defs::rule_class_functions::rule_function_state;
use blaze_protos::stardoc_output::{
    AspectInfo, AttributeInfo, AttributeType, MacroInfo, ModuleExtensionInfo,
    ModuleExtensionTagClassInfo, ModuleInfo, OriginKey, ProviderFieldInfo, ProviderInfo,
    RepositoryRuleInfo, StarlarkOtherSymbolInfo,
};
use blaze_starlark::structs::StructureMarker;
use blaze_starlark::syntax::java::ast::{DocCommentLine, DocComments};
use starlark::environment::FrozenModule;
use starlark::eval::DefModule;
use starlark::values::{Heap, Value};
use starlark_map::small_map::SmallMap;

use crate::attribute_info_extractor::add_documentable_attributes;
use crate::extraction_exception::ExtractionException;
use crate::extractor_context::ExtractorContext;
use crate::label_renderer::LabelRenderer;
use crate::rule_info_extractor::build_rule_info;
use crate::starlark_function_info_extractor::from_name_and_function;

// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:71 ModuleInfoExtractor.IMPLICIT_MACRO_ATTRIBUTES
pub fn implicit_macro_attributes() -> Vec<(&'static str, AttributeInfo)> {
    vec![
        (
            "name",
            AttributeInfo {
                name: "name".to_owned(),
                r#type: AttributeType::Name as i32,
                mandatory: true,
                doc_string: "A unique name for this macro instance. Normally, this is also the \
                             name for the macro's main or only target. The names of any other \
                             targets that this macro might create will be this name with a \
                             string suffix."
                    .to_owned(),
                ..AttributeInfo::default()
            },
        ),
        (
            "visibility",
            AttributeInfo {
                name: "visibility".to_owned(),
                r#type: AttributeType::LabelList as i32,
                mandatory: false,
                nonconfigurable: true,
                natively_defined: true,
                doc_string: "The visibility to be passed to this macro's exported targets. It \
                             always implicitly includes the location where this macro is \
                             instantiated, so this attribute only needs to be explicitly set if \
                             you want the macro's targets to be additionally visible somewhere \
                             else."
                    .to_owned(),
                ..AttributeInfo::default()
            },
        ),
    ]
}

// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:98 ModuleInfoExtractor.IMPLICIT_REPOSITORY_RULE_ATTRIBUTES
pub fn implicit_repository_rule_attributes() -> Vec<(&'static str, AttributeInfo)> {
    vec![(
        "name",
        AttributeInfo {
            name: "name".to_owned(),
            r#type: AttributeType::Name as i32,
            mandatory: true,
            doc_string: "A unique name for this repository.".to_owned(),
            ..AttributeInfo::default()
        },
    )]
}

/// The module being documented: its globals in definition order (Java `Module.getGlobals()`),
/// its docstring and its file name and label (from its `BazelModuleContext`).
pub struct DocumentedModule<'a> {
    pub module: &'a FrozenModule,
    /// `BazelModuleContext.filename()`, for error messages.
    pub filename: Option<String>,
    /// `BazelModuleContext.label()`.
    pub label: Option<blaze_cmdline::Label>,
}

impl DocumentedModule<'_> {
    /// `module.getGlobals()`: the module's global bindings (not its `load` bindings, which
    /// starlark-rust keeps as private names).
    fn globals(&self) -> Vec<(String, starlark::values::FrozenValue)> {
        // FrozenModule::names(): the public names (starlark-rust keeps `load` bindings private).
        self.module
            .names()
            .filter_map(|name| {
                let value = self.module.get_option(name.as_str()).ok().flatten()?;
                // The value is kept alive by `self.module`.
                Some((name.as_str().to_owned(), unsafe {
                    value.unchecked_frozen_value()
                }))
            })
            .collect()
    }
}

/// API documentation extractor for a compiled, loaded Starlark module.
// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:65 ModuleInfoExtractor
pub struct ModuleInfoExtractor<'p> {
    is_wanted_qualified_name: &'p dyn Fn(&str) -> bool,
    label_renderer: LabelRenderer,
    allow_unused_doc_comments: bool,
}

impl<'p> ModuleInfoExtractor<'p> {
    /// Constructs an instance of `ModuleInfoExtractor`.
    ///
    /// - `is_wanted_qualified_name`: a predicate to filter the module's qualified names. A
    ///   qualified name is documented if and only if (1) each component of the qualified name is
    ///   public (in other words, the first character of each component of the qualified name is
    ///   alphabetic) and (2) the qualified name, or one of its ancestor qualified names,
    ///   satisfies the wanted predicate.
    /// - `label_renderer`: a string renderer for labels.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:119 ModuleInfoExtractor.<init>
    pub fn new(
        is_wanted_qualified_name: &'p dyn Fn(&str) -> bool,
        label_renderer: LabelRenderer,
    ) -> Self {
        ModuleInfoExtractor {
            is_wanted_qualified_name,
            label_renderer,
            allow_unused_doc_comments: false,
        }
    }

    /// Allows unused doc comments in modules.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:126 ModuleInfoExtractor.allowUnusedDocComments
    pub fn allow_unused_doc_comments(mut self) -> Self {
        self.allow_unused_doc_comments = true;
        self
    }

    /// Extracts structured documentation for the loadable symbols of a given module.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:132 ModuleInfoExtractor.extractFrom
    pub fn extract_from(
        &self,
        module: &DocumentedModule<'_>,
        doc_comments_map: &SmallMap<String, DocComments>,
        unused_doc_comment_lines: &[DocCommentLine],
    ) -> Result<ModuleInfo, ExtractionException> {
        if !self.allow_unused_doc_comments && !unused_doc_comment_lines.is_empty() {
            return Err(ExtractionException::in_module(
                module.filename.as_deref(),
                format!(
                    "unexpected or conflicting doc comments on line{} {}; a doc comment must be \
                     attached to the declaration of a global variable",
                    if unused_doc_comment_lines.len() > 1 {
                        "s"
                    } else {
                        ""
                    },
                    unused_doc_comment_lines
                        .iter()
                        .map(|c| c.line.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let mut builder = ModuleInfo::default();
        // module.getDocumentation(): the trimmed module docstring.
        if let Some(doc) = module.module.raw_docstring() {
            builder.module_docstring = blaze_packages::starlark_defs::util::trim_doc_string(doc);
        }
        if let Some(label) = &module.label {
            builder.file = self.label_renderer.render(label);
        }

        // We do two traversals over the module's globals: (1) find qualified names (including any
        // nesting structs) for providers loadable from this module; (2) build the documentation
        // proto, using the information from traversal 1 for provider names references by rules
        // and attributes.
        let mut provider_qualified_name_collector = ProviderQualifiedNameCollector {
            qualified_names: Vec::new(),
        };
        traverse(module, &mut provider_qualified_name_collector)?;
        let mut context = ExtractorContext::new(self.label_renderer.clone());
        let mut provider_qualified_names: HashMap<ProviderKey, String> = HashMap::new();
        for (key, name) in provider_qualified_name_collector.qualified_names {
            provider_qualified_names.entry(key).or_insert(name);
        }
        context.provider_qualified_names = provider_qualified_names;
        let mut documentation_extractor = DocumentationExtractor {
            filename: module.filename.clone(),
            doc_comments_map,
            module_info: &mut builder,
            is_wanted_qualified_name: self.is_wanted_qualified_name,
            context,
            allow_unused_doc_comments: self.allow_unused_doc_comments,
        };
        traverse(module, &mut documentation_extractor)?;
        Ok(builder)
    }
}

/// A stateful visitor which traverses a Starlark module's documentable globals, recursing into
/// structs.
// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:197 ModuleInfoExtractor.GlobalsVisitor
trait GlobalsVisitor {
    /// Returns whether the visitor should visit (and possibly recurse into) the value with the
    /// given qualified name.
    fn should_visit(&self, qualified_name: &str) -> bool;

    fn visit_rule(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_macro_function(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_provider(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_function(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_aspect(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_module_extension(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn visit_repository_rule(
        &mut self,
        _qualified_name: &str,
        _value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        Ok(())
    }
    fn maybe_visit_other_symbol(&mut self, _qualified_name: &str, _value: Value<'_>) {}
}

// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:198 GlobalsVisitor.traverse
fn traverse(
    module: &DocumentedModule<'_>,
    visitor: &mut dyn GlobalsVisitor,
) -> Result<(), ExtractionException> {
    Heap::temp(|heap| {
        for (global_symbol, value) in module.globals() {
            if ExtractorContext::is_public_name(&global_symbol) {
                maybe_visit(
                    visitor,
                    heap,
                    &global_symbol,
                    value.to_value(),
                    /* should_visit_verified_for_ancestor= */ false,
                )?;
            }
        }
        Ok(())
    })
}

/// Java `value instanceof StarlarkFunction`: a `def` or `lambda`.
fn is_starlark_function(value: Value<'_>) -> bool {
    DefModule::def_name(value).is_some()
}

/// Java `value instanceof Structure`.
fn is_structure(value: Value<'_>) -> bool {
    value.request_value::<StructureMarker>().is_some()
}

/// Java `value instanceof StarlarkExportable && !exportable.isExported()`, for the exportable
/// values this extractor recognizes.
fn is_unexported_exportable(value: Value<'_>) -> bool {
    if let Some(state) = rule_function_state(value) {
        return !state.is_exported();
    }
    if let Some(state) = macro_function_state(value) {
        return !state.is_exported();
    }
    if let Some(provider) = StarlarkProvider::from_value(value) {
        return !provider.is_exported();
    }
    if let Some(aspect) = aspect_data(value) {
        return !aspect.is_exported();
    }
    if let Some(repo_rule) = documentable_repo_rule(value) {
        return !repo_rule.exported;
    }
    false
}

/// - `qualified_name`: the name under which the value may be accessed by a user of the module;
///   for example, "foo.bar" for field bar of global struct foo
/// - `should_visit_verified_for_ancestor`: whether `should_visit` was verified true for an
///   ancestor struct's qualified name
// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:225 GlobalsVisitor.maybeVisit
fn maybe_visit<'v>(
    visitor: &mut dyn GlobalsVisitor,
    heap: Heap<'v>,
    qualified_name: &str,
    value: Value<'v>,
    should_visit_verified_for_ancestor: bool,
) -> Result<(), ExtractionException> {
    if should_visit_verified_for_ancestor || visitor.should_visit(qualified_name) {
        if is_unexported_exportable(value) {
            // Unexported StarlarkExportables are not usable and therefore do not need to have
            // docs generated.
            return Ok(());
        }
        if rule_function_state(value).is_some() {
            visitor.visit_rule(qualified_name, value)?;
        } else if macro_function_state(value).is_some() {
            visitor.visit_macro_function(qualified_name, value)?;
        } else if StarlarkProvider::from_value(value).is_some() {
            visitor.visit_provider(qualified_name, value)?;
        } else if is_starlark_function(value) {
            visitor.visit_function(qualified_name, value)?;
        } else if aspect_data(value).is_some() {
            visitor.visit_aspect(qualified_name, value)?;
        } else if documentable_repo_rule(value).is_some() {
            visitor.visit_repository_rule(qualified_name, value)?;
        } else if documentable_module_extension(value).is_some() {
            visitor.visit_module_extension(qualified_name, value)?;
        } else {
            visitor.maybe_visit_other_symbol(qualified_name, value);
            if is_structure(value) {
                recurse_into_structure(
                    visitor,
                    heap,
                    qualified_name,
                    value,
                    /* should_visit_verified_for_ancestor= */ true,
                )?;
            }
        }
    } else if is_structure(value) {
        recurse_into_structure(
            visitor,
            heap,
            qualified_name,
            value,
            /* should_visit_verified_for_ancestor= */ false,
        )?;
    }
    // TODO(b/276733504): should we recurse into dicts to search for documentable values? Note
    // that dicts (unlike structs!) can have reference cycles, so we would need to track the set
    // of traversed entities.
    Ok(())
}

// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:302 GlobalsVisitor.recurseIntoStructure
fn recurse_into_structure<'v>(
    visitor: &mut dyn GlobalsVisitor,
    heap: Heap<'v>,
    qualified_name: &str,
    structure: Value<'v>,
    should_visit_verified_for_ancestor: bool,
) -> Result<(), ExtractionException> {
    // structure.getFieldNames(), structure.getValue(fieldName)
    // PORT: the fields are read with `dir_attr` / `get_attr` (Blaze's structs never fail to read
    // a listed field; Java's "failed to read value" error has no counterpart).
    for field_name in structure.dir_attr() {
        if ExtractorContext::is_public_name(&field_name)
            && let Some(field_value) = structure.get_attr(&field_name, heap).ok().flatten()
        {
            maybe_visit(
                visitor,
                heap,
                &format!("{qualified_name}.{field_name}"),
                field_value,
                should_visit_verified_for_ancestor,
            )?;
        }
    }
    Ok(())
}

/// A `GlobalsVisitor` which finds the qualified names (including any nesting structs) for
/// providers loadable from this module.
// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:332 ModuleInfoExtractor.ProviderQualifiedNameCollector
struct ProviderQualifiedNameCollector {
    /// If the same provider is accessible under multiple names, the first documentable name
    /// wins (`putIfAbsent` when the map is built).
    qualified_names: Vec<(ProviderKey, String)>,
}

impl GlobalsVisitor for ProviderQualifiedNameCollector {
    /// Returns true always: a non-wanted provider symbol may still be referred to by a wanted
    /// rule.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:363 ProviderQualifiedNameCollector.shouldVisit
    fn should_visit(&self, _qualified_name: &str) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:368 ProviderQualifiedNameCollector.visitProvider
    fn visit_provider(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        let provider = StarlarkProvider::from_value(value).expect("a provider");
        self.qualified_names
            .push((provider.get_key().clone(), qualified_name.to_owned()));
        Ok(())
    }
}

/// A `GlobalsVisitor` which extracts documentation for symbols in this module.
// Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:375 ModuleInfoExtractor.DocumentationExtractor
struct DocumentationExtractor<'a, 'p> {
    filename: Option<String>,
    doc_comments_map: &'a SmallMap<String, DocComments>,
    module_info: &'a mut ModuleInfo,
    is_wanted_qualified_name: &'p dyn Fn(&str) -> bool,
    context: ExtractorContext,
    allow_unused_doc_comments: bool,
}

impl DocumentationExtractor<'_, '_> {
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:421 DocumentationExtractor.checkNoDocComments
    fn check_no_doc_comments(
        &self,
        qualified_name: &str,
        what: &str,
        expected: &str,
    ) -> Result<(), ExtractionException> {
        if let Some(doc_comments) = self.doc_comments_map.get(qualified_name)
            && !self.allow_unused_doc_comments
        {
            return Err(ExtractionException::in_module(
                self.filename.as_deref(),
                format!(
                    "unexpected doc comment for {} on line {}; API documentation for a {} must \
                     be provided in {}",
                    qualified_name,
                    doc_comments.start_line(),
                    what,
                    expected
                ),
            ));
        }
        Ok(())
    }
}

impl GlobalsVisitor for DocumentationExtractor<'_, '_> {
    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:416 DocumentationExtractor.shouldVisit
    fn should_visit(&self, qualified_name: &str) -> bool {
        (self.is_wanted_qualified_name)(qualified_name)
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:435 DocumentationExtractor.visitFunction
    fn visit_function(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(
            qualified_name,
            "function",
            "a docstring at the top of the function body",
        )?;
        let info = from_name_and_function(qualified_name, value, &self.context.label_renderer)?;
        self.module_info.func_info.push(info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:442 DocumentationExtractor.visitRule
    fn visit_rule(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(qualified_name, "rule", "the doc argument to rule()")?;
        let state = rule_function_state(value).expect("a rule function");
        let rule_class = state
            .rule_class()
            .expect("an exported rule has a rule class");
        let info = build_rule_info(&self.context, qualified_name, rule_class);
        self.module_info.rule_info.push(info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:450 DocumentationExtractor.visitMacroFunction
    fn visit_macro_function(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(qualified_name, "macro", "the doc argument to macro()")?;
        let macro_function = macro_function_state(value).expect("a macro function");
        let mut macro_info = MacroInfo {
            // Record the name under which this symbol is made accessible, which may differ from
            // the symbol's exported name
            macro_name: qualified_name.to_owned(),
            // ... but record the origin rule key for cross references.
            origin_key: Some(OriginKey {
                name: macro_function.get_name(),
                file: self.context.label_renderer.render(
                    macro_function
                        .get_extension_label()
                        .expect("an exported macro has an extension label"),
                ),
            }),
            ..MacroInfo::default()
        };
        if let Some(doc) = macro_function.get_documentation() {
            macro_info.doc_string = doc.to_owned();
        }
        let macro_class = macro_function
            .get_macro_class()
            .expect("an exported macro has a macro class");
        if macro_class.is_finalizer() {
            macro_info.finalizer = true;
        }
        // For symbolic macros, always extract non-Starlark attributes (to support
        // inherit_attrs).
        let mut context_for_implicit_macro_attributes = self.context.clone();
        context_for_implicit_macro_attributes.extract_natively_defined_attrs = true;
        add_documentable_attributes(
            &context_for_implicit_macro_attributes,
            &implicit_macro_attributes(),
            macro_class.get_attribute_provider().get_attributes(),
            &mut macro_info.attribute,
        );
        self.module_info.macro_info.push(macro_info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:488 DocumentationExtractor.visitProvider
    fn visit_provider(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(qualified_name, "provider", "the doc argument to provider()")?;
        let provider = StarlarkProvider::from_value(value).expect("a provider");
        let mut provider_info = ProviderInfo {
            // Record the name under which this symbol is made accessible, which may differ from
            // the symbol's exported name.
            provider_name: qualified_name.to_owned(),
            ..ProviderInfo::default()
        };
        // Record the origin provider key for cross references.
        provider_info.origin_key = Some(OriginKey {
            name: provider.get_name(),
            file: self.context.label_renderer.render(
                provider
                    .get_key()
                    .get_extension_label()
                    .expect("a Starlark provider key has an extension label"),
            ),
        });
        if let Some(doc) = provider.get_documentation() {
            provider_info.doc_string = doc.to_owned();
        }
        if let Some(schema) = provider.get_schema() {
            for (field_name, field_doc) in schema {
                if ExtractorContext::is_public_name(field_name) {
                    provider_info.field_info.push(ProviderFieldInfo {
                        name: field_name.clone(),
                        doc_string: field_doc.clone().unwrap_or_default(),
                    });
                }
            }
        }
        // TODO(b/276733504): if init is a dict-returning native method (e.g. `dict`), do we
        // document it?
        if let Some(init) = provider.get_init()
            && is_starlark_function(init)
        {
            provider_info.init = Some(from_name_and_function(
                qualified_name,
                init,
                &self.context.label_renderer,
            )?);
        }
        self.module_info.provider_info.push(provider_info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:542 DocumentationExtractor.visitAspect
    fn visit_aspect(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(qualified_name, "aspect", "the doc argument to aspect()")?;
        let aspect = aspect_data(value).expect("an aspect");
        let aspect_class = aspect.get_aspect_class();
        let mut aspect_info = AspectInfo {
            // Record the name under which this symbol is made accessible, which may differ from
            // the symbol's exported name
            aspect_name: qualified_name.to_owned(),
            // ... but record the origin aspect key for cross references.
            origin_key: Some(OriginKey {
                name: aspect_class.get_exported_name().to_owned(),
                file: self
                    .context
                    .label_renderer
                    .render(aspect_class.get_extension_label()),
            }),
            ..AspectInfo::default()
        };
        if let Some(doc) = aspect.get_documentation() {
            aspect_info.doc_string = doc.to_owned();
        }
        match aspect.get_attribute_aspects() {
            AspectPropagationEdgesSupplier::FixedList(s) => {
                for aspect_attribute in s.get_list() {
                    if ExtractorContext::is_public_name(aspect_attribute) {
                        aspect_info.aspect_attribute.push(aspect_attribute.clone());
                    }
                }
            }
            AspectPropagationEdgesSupplier::Function(_) => {
                // TODO(b/394400334): Make {@code attr_aspects} function avaiable in {@code
                // AspectInfo}.
            }
        }
        add_documentable_attributes(
            &self.context,
            &[],
            aspect.get_attributes(),
            &mut aspect_info.attribute,
        );
        self.module_info.aspect_info.push(aspect_info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:581 DocumentationExtractor.visitModuleExtension
    fn visit_module_extension(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(
            qualified_name,
            "module extension",
            "the doc argument to module_extension()",
        )?;
        let module_extension = documentable_module_extension(value).expect("a module extension");
        let mut module_extension_info = ModuleExtensionInfo {
            extension_name: qualified_name.to_owned(),
            // TODO(arostovtsev): attempt to retrieve the name under which the module was
            // originally defined so we can call setName() too.
            origin_key: Some(OriginKey {
                file: self
                    .context
                    .label_renderer
                    .render(&module_extension.defining_bzl_file_label),
                ..OriginKey::default()
            }),
            ..ModuleExtensionInfo::default()
        };
        if let Some(doc) = &module_extension.doc {
            module_extension_info.doc_string = doc.clone();
        }
        for tag_class in &module_extension.tag_classes {
            let mut tag_class_info = ModuleExtensionTagClassInfo {
                tag_name: tag_class.name.clone(),
                ..ModuleExtensionTagClassInfo::default()
            };
            if let Some(doc) = &tag_class.doc {
                tag_class_info.doc_string = doc.clone();
            }
            add_documentable_attributes(
                &self.context,
                &[],
                &tag_class.attributes,
                &mut tag_class_info.attribute,
            );
            module_extension_info.tag_class.push(tag_class_info);
        }
        self.module_info
            .module_extension_info
            .push(module_extension_info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:622 DocumentationExtractor.visitRepositoryRule
    fn visit_repository_rule(
        &mut self,
        qualified_name: &str,
        value: Value<'_>,
    ) -> Result<(), ExtractionException> {
        self.check_no_doc_comments(
            qualified_name,
            "repository rule",
            "the doc argument to repository_rule()",
        )?;
        let repo_rule = documentable_repo_rule(value).expect("a repository rule");
        let mut repository_rule_info = RepositoryRuleInfo {
            rule_name: qualified_name.to_owned(),
            ..RepositoryRuleInfo::default()
        };
        if let Some(doc) = &repo_rule.doc {
            repository_rule_info.doc_string = doc.clone();
        }
        repository_rule_info.origin_key = Some(OriginKey {
            name: repo_rule
                .rule_name
                .clone()
                .expect("an exported repository rule has a name"),
            file: self
                .context
                .label_renderer
                .render(&repo_rule.bzl_file_label),
        });
        add_documentable_attributes(
            &self.context,
            &implicit_repository_rule_attributes(),
            &repo_rule.attributes,
            &mut repository_rule_info.attribute,
        );
        for env in &repo_rule.environ {
            repository_rule_info.environ.push(env.clone());
        }
        self.module_info
            .repository_rule_info
            .push(repository_rule_info);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/ModuleInfoExtractor.java:648 DocumentationExtractor.maybeVisitOtherSymbol
    fn maybe_visit_other_symbol(&mut self, qualified_name: &str, value: Value<'_>) {
        let Some(doc_comments) = self.doc_comments_map.get(qualified_name) else {
            // Don't emit documentation for symbols without doc comments.
            return;
        };
        self.module_info
            .starlark_other_symbol_info
            .push(StarlarkOtherSymbolInfo {
                name: qualified_name.to_owned(),
                doc: doc_comments.get_text(),
                // Starlark.type(value)
                type_name: value.get_type().to_owned(),
            });
    }
}
