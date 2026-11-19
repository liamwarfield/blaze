//! `rules/java/JavaOutput.java`: a collection of artifacts associated with a jar output, and
//! `rules/java/JavaCompileOutputs.java`: the outputs of a `JavaCompileAction`.
//!
//! PORT: Bazel 9 creates `JavaOutput` instances in Starlark (rules_java); the core only reads
//! them (`JavaPluginInfo.wrap`), so the native value is a plain record without a Starlark type.

use blaze_actions::Artifact;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use starlark::values::{Heap, Value};

use super::provider_util::{is_struct_impl, starlark_type};
use super::starlark_info_wrapper::{StarlarkInfoWrapper, struct_get_value};
use crate::cpp::util::{errorf, sequence_cast_artifacts};

/// The outputs of a `JavaCompileAction`.
// Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:22 JavaCompileOutputs
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JavaCompileOutputs {
    /// The class jar Artifact to create with the Action.
    output: Artifact,
    /// The output artifact for the manifest proto emitted from JavaBuilder.
    manifest_proto: Option<Artifact>,
    deps_proto: Option<Artifact>,
    /// The generated class jar, or `None` if no annotation processing is expected.
    gen_class: Option<Artifact>,
    /// The generated sources jar Artifact to create with the Action (`None` if no sources will
    /// be generated).
    gen_source: Option<Artifact>,
    /// An archive of generated native header files.
    native_header: Option<Artifact>,
}

impl JavaCompileOutputs {
    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:26 JavaCompileOutputs.output
    pub fn output(&self) -> &Artifact {
        &self.output
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:29 JavaCompileOutputs.manifestProto
    pub fn manifest_proto(&self) -> Option<&Artifact> {
        self.manifest_proto.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:32 JavaCompileOutputs.depsProto
    pub fn deps_proto(&self) -> Option<&Artifact> {
        self.deps_proto.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:37 JavaCompileOutputs.genClass
    pub fn gen_class(&self) -> Option<&Artifact> {
        self.gen_class.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:44 JavaCompileOutputs.genSource
    pub fn gen_source(&self) -> Option<&Artifact> {
        self.gen_source.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:48 JavaCompileOutputs.nativeHeader
    pub fn native_header(&self) -> Option<&Artifact> {
        self.native_header.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:50 JavaCompileOutputs.builder
    pub fn builder() -> JavaCompileOutputsBuilder {
        JavaCompileOutputsBuilder::default()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:54 JavaCompileOutputs.toBuilder
    pub fn to_builder(&self) -> JavaCompileOutputsBuilder {
        JavaCompileOutputsBuilder {
            output: Some(self.output.clone()),
            manifest_proto: self.manifest_proto.clone(),
            deps_proto: self.deps_proto.clone(),
            gen_class: self.gen_class.clone(),
            gen_source: self.gen_source.clone(),
            native_header: self.native_header.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:56 JavaCompileOutputs.withOutput
    pub fn with_output(&self, output: Artifact) -> JavaCompileOutputs {
        let mut b = self.to_builder();
        b.output(output);
        b.build()
    }
}

/// `JavaCompileOutputs.Builder` (AutoValue).
// Java: com/google/devtools/build/lib/rules/java/JavaCompileOutputs.java:60 JavaCompileOutputs.Builder
#[derive(Default)]
pub struct JavaCompileOutputsBuilder {
    output: Option<Artifact>,
    manifest_proto: Option<Artifact>,
    deps_proto: Option<Artifact>,
    gen_class: Option<Artifact>,
    gen_source: Option<Artifact>,
    native_header: Option<Artifact>,
}

impl JavaCompileOutputsBuilder {
    pub fn output(&mut self, artifact: Artifact) -> &mut Self {
        self.output = Some(artifact);
        self
    }

    /// PORT: AutoValue rejects a null `manifestProto` at `build()` (the property is not
    /// `@Nullable`); `None` here fails there the same way.
    pub fn manifest_proto(&mut self, artifact: Option<Artifact>) -> &mut Self {
        self.manifest_proto = artifact;
        self
    }

    pub fn deps_proto(&mut self, artifact: Option<Artifact>) -> &mut Self {
        self.deps_proto = artifact;
        self
    }

    pub fn gen_class(&mut self, artifact: Option<Artifact>) -> &mut Self {
        self.gen_class = artifact;
        self
    }

    pub fn gen_source(&mut self, artifact: Option<Artifact>) -> &mut Self {
        self.gen_source = artifact;
        self
    }

    pub fn native_header(&mut self, artifact: Option<Artifact>) -> &mut Self {
        self.native_header = artifact;
        self
    }

    pub fn build(&self) -> JavaCompileOutputs {
        let output = self.output.clone().unwrap_or_else(|| {
            panic!("IllegalStateException: Missing required properties: output")
        });
        if self.manifest_proto.is_none() {
            panic!("NullPointerException: Null manifestProto");
        }
        JavaCompileOutputs {
            output,
            manifest_proto: self.manifest_proto.clone(),
            deps_proto: self.deps_proto.clone(),
            gen_class: self.gen_class.clone(),
            gen_source: self.gen_source.clone(),
            native_header: self.native_header.clone(),
        }
    }
}

/// A collection of artifacts associated with a jar output.
// Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:44 JavaOutput
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JavaOutput {
    class_jar: Artifact,
    compile_jar: Option<Artifact>,
    header_compilation_jar: Option<Artifact>,
    compile_jdeps: Option<Artifact>,
    generated_class_jar: Option<Artifact>,
    generated_source_jar: Option<Artifact>,
    native_headers_jar: Option<Artifact>,
    manifest_proto: Option<Artifact>,
    jdeps: Option<Artifact>,
    /// Sources archive files.
    source_jars: NestedSet<Artifact>,
}

impl JavaOutput {
    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:62 JavaOutput.getClassJar
    pub fn get_class_jar(&self) -> &Artifact {
        &self.class_jar
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:68 JavaOutput.getCompileJar
    pub fn get_compile_jar(&self) -> Option<&Artifact> {
        self.compile_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:74 JavaOutput.getHeaderCompilationJar
    pub fn get_header_compilation_jar(&self) -> Option<&Artifact> {
        self.header_compilation_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:80 JavaOutput.getCompileJdeps
    pub fn get_compile_jdeps(&self) -> Option<&Artifact> {
        self.compile_jdeps.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:86 JavaOutput.getGeneratedClassJar
    pub fn get_generated_class_jar(&self) -> Option<&Artifact> {
        self.generated_class_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:92 JavaOutput.getGeneratedSourceJar
    pub fn get_generated_source_jar(&self) -> Option<&Artifact> {
        self.generated_source_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:98 JavaOutput.getNativeHeadersJar
    pub fn get_native_headers_jar(&self) -> Option<&Artifact> {
        self.native_headers_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:104 JavaOutput.getManifestProto
    pub fn get_manifest_proto(&self) -> Option<&Artifact> {
        self.manifest_proto.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:110 JavaOutput.getJdeps
    pub fn get_jdeps(&self) -> Option<&Artifact> {
        self.jdeps.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:150 JavaOutput.getIJar
    pub fn get_i_jar(&self) -> Option<&Artifact> {
        self.compile_jar.as_ref()
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:157 JavaOutput.getSrcJar
    pub fn get_src_jar(&self) -> Option<Artifact> {
        let list = self.get_source_jars_as_list();
        match list.len() {
            0 => None,
            1 => Some(list[0].clone()),
            _ => panic!(
                "IllegalArgumentException: expected one element but was: {:?}",
                list.iter().map(|a| a.to_string()).collect::<Vec<_>>()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:161 JavaOutput.getSourceJarsAsList
    pub fn get_source_jars_as_list(&self) -> Vec<Artifact> {
        self.source_jars.to_list().to_vec()
    }

    pub fn source_jars(&self) -> &NestedSet<Artifact> {
        &self.source_jars
    }

    /// Translates a collection of `JavaOutput` for use in native code.
    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:127 JavaOutput.wrapSequence
    pub fn wrap_sequence<'v>(
        outputs: &[Value<'v>],
        heap: Heap<'v>,
    ) -> starlark::Result<Vec<JavaOutput>> {
        let mut result = Vec::with_capacity(outputs.len());
        for info in outputs {
            if is_struct_impl(*info) {
                result.push(Self::from_starlark_java_output(*info, heap)?);
            } else {
                return Err(errorf(format!(
                    "expected JavaOutput, got: {}",
                    starlark_type(*info)
                )));
            }
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:171 JavaOutput.fromStarlarkJavaOutput
    pub fn from_starlark_java_output<'v>(
        r#struct: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<JavaOutput> {
        let starlark_source_jars = struct_get_value(r#struct, "source_jars", heap);
        let source_jars = match starlark_source_jars {
            // Java: Depset.noneableCast(null) is a NullPointerException-free cast of a missing
            // field; StructImpl.getValue returns null, which is neither NONE nor a Depset.
            Some(v) if v.is_none() => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            Some(v) if v.get_type() == "depset" => {
                blaze_analysis::rule::depsets::depset_to_artifacts(v, "source_jars")?
            }
            Some(v) => NestedSetBuilder::wrap(
                Order::STABLE_ORDER,
                sequence_cast_artifacts(v, "source_jars", heap)?,
            ),
            None => panic!("NullPointerException: source_jars"),
        };
        let w = StarlarkInfoWrapper::new(r#struct, heap);
        let mut b = JavaOutputBuilder::default();
        b.class_jar = Self::null_if_none(&w, "class_jar")?;
        b.compile_jar = Self::null_if_none(&w, "compile_jar")?;
        b.header_compilation_jar = Self::null_if_none(&w, "header_compilation_jar")?;
        b.compile_jdeps = Self::null_if_none(&w, "compile_jdeps")?;
        b.generated_class_jar = Self::null_if_none(&w, "generated_class_jar")?;
        b.generated_source_jar = Self::null_if_none(&w, "generated_source_jar")?;
        b.native_headers_jar = Self::null_if_none(&w, "native_headers_jar")?;
        b.manifest_proto = Self::null_if_none(&w, "manifest_proto")?;
        b.jdeps = Self::null_if_none(&w, "jdeps")?;
        b.add_source_jars(&source_jars);
        Ok(b.build())
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:197 JavaOutput.nullIfNone
    fn null_if_none(w: &StarlarkInfoWrapper<'_>, key: &str) -> starlark::Result<Option<Artifact>> {
        match struct_get_value(w.underlying, key, w.heap) {
            Some(v) if !v.is_none() => {
                match blaze_actions::analysis::starlark::starlark_file::artifact_from_value(v) {
                    Some(a) => Ok(Some(a)),
                    None => panic!(
                        "ClassCastException: {} cannot be cast to Artifact",
                        v.get_type()
                    ),
                }
            }
            _ => Ok(None),
        }
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:265 JavaOutput.builder
    pub fn builder() -> JavaOutputBuilder {
        JavaOutputBuilder::default()
    }
}

/// Builder for OutputJar.
// Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:203 JavaOutput.Builder
#[derive(Default)]
pub struct JavaOutputBuilder {
    pub class_jar: Option<Artifact>,
    pub compile_jar: Option<Artifact>,
    pub header_compilation_jar: Option<Artifact>,
    pub compile_jdeps: Option<Artifact>,
    pub generated_class_jar: Option<Artifact>,
    pub generated_source_jar: Option<Artifact>,
    pub native_headers_jar: Option<Artifact>,
    pub manifest_proto: Option<Artifact>,
    pub jdeps: Option<Artifact>,
    source_jars_builder: Option<NestedSetBuilder<Artifact>>,
}

impl JavaOutputBuilder {
    fn source_jars_builder(&mut self) -> &mut NestedSetBuilder<Artifact> {
        self.source_jars_builder
            .get_or_insert_with(NestedSetBuilder::stable_order)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:227 JavaOutput.Builder.addSourceJar
    pub fn add_source_jar(&mut self, value: Option<Artifact>) -> &mut Self {
        if let Some(v) = value {
            self.source_jars_builder().add(v);
        }
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:234 JavaOutput.Builder.addSourceJars
    pub fn add_source_jars(&mut self, values: &NestedSet<Artifact>) -> &mut Self {
        self.source_jars_builder().add_transitive(values);
        self
    }

    /// Populates the builder with outputs from `JavaCompileOutputs`.
    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:240 JavaOutput.Builder.fromJavaCompileOutputs
    pub fn from_java_compile_outputs(&mut self, value: &JavaCompileOutputs) -> &mut Self {
        self.from_java_compile_outputs_with(value, true)
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:245 JavaOutput.Builder.fromJavaCompileOutputs
    pub fn from_java_compile_outputs_with(
        &mut self,
        value: &JavaCompileOutputs,
        include_jdeps: bool,
    ) -> &mut Self {
        self.class_jar = Some(value.output().clone());
        if include_jdeps {
            self.jdeps = value.deps_proto().cloned();
        }
        self.generated_class_jar = value.gen_class().cloned();
        self.generated_source_jar = value.gen_source().cloned();
        self.native_headers_jar = value.native_header().cloned();
        self.manifest_proto = value.manifest_proto().cloned();
        self
    }

    // Java: com/google/devtools/build/lib/rules/java/JavaOutput.java:259 JavaOutput.Builder.build
    pub fn build(&mut self) -> JavaOutput {
        let source_jars = self.source_jars_builder().build();
        // Java: JavaOutput's compact constructor: requireNonNull(classJar, "classJar").
        let class_jar = self
            .class_jar
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: classJar"));
        JavaOutput {
            class_jar,
            compile_jar: self.compile_jar.clone(),
            header_compilation_jar: self.header_compilation_jar.clone(),
            compile_jdeps: self.compile_jdeps.clone(),
            generated_class_jar: self.generated_class_jar.clone(),
            generated_source_jar: self.generated_source_jar.clone(),
            native_headers_jar: self.native_headers_jar.clone(),
            manifest_proto: self.manifest_proto.clone(),
            jdeps: self.jdeps.clone(),
            source_jars,
        }
    }
}
