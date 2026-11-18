//! The Starlark value of the `proto` fragment (`rules/proto/ProtoConfiguration.java`,
//! `starlarkbuildapi/ProtoConfigurationApi.java`).

use blaze_build_config::fragments::proto_configuration::{PROTO_CONFIGURATION, ProtoConfiguration};
use blaze_packages::builtins::java_common::fail_if_called_outside_default_allowlist;
use starlark::environment::MethodsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;

use super::fragment_value;

fragment_value!(
    /// `ctx.fragments.proto`: a configuration fragment representing protocol buffers.
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:45 ProtoConfiguration
    ProtoConfigurationValue,
    ProtoConfiguration,
    "proto",
    "com.google.devtools.build.lib.rules.proto.ProtoConfiguration",
    proto_configuration_methods
);

pub(super) fn register() {
    super::register(&PROTO_CONFIGURATION, ProtoConfigurationValue::alloc);
}

#[starlark_module]
fn proto_configuration_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:170 ProtoConfiguration.protocOptsForStarlark
    #[starlark(attribute)]
    fn experimental_protoc_opts(this: &ProtoConfigurationValue) -> starlark::Result<Vec<String>> {
        Ok(this.fragment().protoc_opts_for_starlark().to_vec())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:178 ProtoConfiguration.experimentalProtoDescriptorSetsIncludeSourceInfoForStarlark
    fn experimental_proto_descriptorsets_include_source_info<'v>(
        this: &ProtoConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        fail_if_called_outside_default_allowlist(eval).map_err(starlark::Error::new_other)?;
        Ok(this
            .fragment()
            .experimental_proto_descriptor_sets_include_source_info())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:234 ProtoConfiguration.strictProtoDepsForStarlark
    fn strict_proto_deps<'v>(
        this: &ProtoConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        fail_if_called_outside_default_allowlist(eval).map_err(starlark::Error::new_other)?;
        Ok(this.fragment().strict_proto_deps_for_starlark())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:240 ProtoConfiguration.strictPublicImportsForStarlark
    fn strict_public_imports<'v>(
        this: &ProtoConfigurationValue,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<String> {
        fail_if_called_outside_default_allowlist(eval).map_err(starlark::Error::new_other)?;
        Ok(this.fragment().strict_public_imports_for_starlark())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:246 ProtoConfiguration.ccProtoLibraryHeaderSuffixesForStarlark
    #[starlark(attribute)]
    fn cc_proto_library_header_suffixes(
        this: &ProtoConfigurationValue,
    ) -> starlark::Result<Vec<String>> {
        Ok(this
            .fragment()
            .cc_proto_library_header_suffixes_for_starlark()
            .to_vec())
    }

    // Java: com/google/devtools/build/lib/rules/proto/ProtoConfiguration.java:251 ProtoConfiguration.ccProtoLibrarySourceSuffixesForStarlark
    #[starlark(attribute)]
    fn cc_proto_library_source_suffixes(
        this: &ProtoConfigurationValue,
    ) -> starlark::Result<Vec<String>> {
        Ok(this
            .fragment()
            .cc_proto_library_source_suffixes_for_starlark()
            .to_vec())
    }
}
