//! Port of `rules/proto/ProtoConfiguration.java` (the fragment; its options are
//! `blaze_optdefs::fragment::proto_configuration`).

use std::any::Any;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_optdefs::fragment::proto_configuration::ProtoConfigurationOptions;
use blaze_options::OptionScalar;

use crate::build_options::BuildOptions;
use crate::fragment::{
    Fragment, FragmentClass, FragmentClassInfo, InvalidConfigurationException, required,
};
use crate::fragment_options_class::PROTO_OPTIONS;

// Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:45 ProtoConfiguration
/// Configuration for Protocol Buffer Libraries.
#[derive(Debug)]
pub struct ProtoConfiguration {
    protoc_opts: Vec<String>,
    cc_proto_library_header_suffixes: Vec<String>,
    cc_proto_library_source_suffixes: Vec<String>,
    options: ProtoConfigurationOptions,
}

/// `ProtoConfiguration.class`.
pub static PROTO_CONFIGURATION: FragmentClassInfo = FragmentClassInfo {
    java_name: "com.google.devtools.build.lib.rules.proto.ProtoConfiguration",
    starlark_name: Some("proto"),
    required_options: || vec![&PROTO_OPTIONS],
    requires_starlark_options: false,
    constructor: create,
};

fn create(options: &BuildOptions) -> Result<Arc<dyn Fragment>, InvalidConfigurationException> {
    let fragment: Arc<dyn Fragment> = Arc::new(ProtoConfiguration::new(options));
    Ok(fragment)
}

impl ProtoConfiguration {
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:161 ProtoConfiguration.<init>
    pub fn new(build_options: &BuildOptions) -> ProtoConfiguration {
        let options = required::<ProtoConfigurationOptions>(build_options);
        ProtoConfiguration {
            protoc_opts: options.protoc_opts.clone(),
            cc_proto_library_header_suffixes: options.cc_proto_library_header_suffixes.clone(),
            cc_proto_library_source_suffixes: options.cc_proto_library_source_suffixes.clone(),
            options: options.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:170 ProtoConfiguration.protocOptsForStarlark
    pub fn protoc_opts_for_starlark(&self) -> &[String] {
        &self.protoc_opts
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:184 ProtoConfiguration.experimentalProtoDescriptorSetsIncludeSourceInfo
    pub fn experimental_proto_descriptor_sets_include_source_info(&self) -> bool {
        self.options
            .experimental_proto_descriptor_sets_include_source_info
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:193 ProtoConfiguration.protoCompiler
    pub fn proto_compiler(&self) -> Option<&Label> {
        Some(self.options.proto_compiler.get())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:202 ProtoConfiguration.protoToolchainForJava
    pub fn proto_toolchain_for_java(&self) -> Option<&Label> {
        self.options
            .proto_toolchain_for_java
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:211 ProtoConfiguration.protoToolchainForJ2objc
    pub fn proto_toolchain_for_j2objc(&self) -> Option<&Label> {
        self.options
            .proto_toolchain_for_j2objc
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:220 ProtoConfiguration.protoToolchainForJavaLite
    pub fn proto_toolchain_for_java_lite(&self) -> Option<&Label> {
        Some(self.options.proto_toolchain_for_java_lite.get())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:229 ProtoConfiguration.protoToolchainForCc
    pub fn proto_toolchain_for_cc(&self) -> Option<&Label> {
        self.options
            .proto_toolchain_for_cc
            .as_ref()
            .map(|l| l.get())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:234 ProtoConfiguration.strictProtoDepsForStarlark
    /// `options.strictProtoDeps.toString()`.
    // PORT: the Starlark method's BuiltinRestriction check belongs to the Starlark wrapper.
    pub fn strict_proto_deps_for_starlark(&self) -> String {
        self.options.strict_proto_deps.java_to_string()
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:240 ProtoConfiguration.strictPublicImportsForStarlark
    /// `options.strictPublicImports.toString()`.
    // PORT: the Starlark method's BuiltinRestriction check belongs to the Starlark wrapper.
    pub fn strict_public_imports_for_starlark(&self) -> String {
        self.options.strict_public_imports.java_to_string()
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:246 ProtoConfiguration.ccProtoLibraryHeaderSuffixesForStarlark
    pub fn cc_proto_library_header_suffixes_for_starlark(&self) -> &[String] {
        &self.cc_proto_library_header_suffixes
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:251 ProtoConfiguration.ccProtoLibrarySourceSuffixesForStarlark
    pub fn cc_proto_library_source_suffixes_for_starlark(&self) -> &[String] {
        &self.cc_proto_library_source_suffixes
    }
}

impl Fragment for ProtoConfiguration {
    fn fragment_class(&self) -> FragmentClass {
        &PROTO_CONFIGURATION
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
