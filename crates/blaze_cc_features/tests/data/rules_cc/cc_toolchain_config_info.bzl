# Copyright 2025 The Bazel Authors. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""rules_cc 0.2.17 create_cc_toolchain_config_info, returning a struct (test harness).

Blaze test data: the other files in this directory (cc_toolchain_config_lib.bzl, action_names.bzl,
legacy_features.bzl) are unchanged copies from rules_cc 0.2.17. This file is rules_cc's
cc/private/toolchain_config/cc_toolchain_config_info.bzl with the provider replaced by a struct and
skylib's paths.join and cc_internal.exec_os inlined.
"""

load(
    "//cc/private/toolchain_config:legacy_features.bzl",
    "get_features_to_appear_last",
    "get_legacy_action_configs",
    "get_legacy_features",
)

# bazel_skylib paths.join
def _paths_join(path, *others):
    result = path
    for p in others:
        if p.startswith("/"):
            result = p
        elif not result or result.endswith("/"):
            result += p
        else:
            result += "/" + p
    return result

# buildifier: disable=function-docstring
def create_cc_toolchain_config_info(
        *,
        ctx,
        toolchain_identifier,
        compiler,
        features = [],
        action_configs = [],
        artifact_name_patterns = [],
        cxx_builtin_include_directories = [],
        # buildifier: disable=unused-variable
        host_system_name = None,
        target_system_name = None,
        target_cpu = None,
        target_libc = None,
        abi_version = None,
        abi_libc_version = None,
        tool_paths = [],
        make_variables = [],
        builtin_sysroot = None,
        # buildifier: disable=unused-variable
        cc_target_os = None):
    feature_names = set([f.name for f in features])
    action_config_names = set([a.action_name for a in action_configs])
    if "no_legacy_features" not in feature_names:
        gcc_tool_path = "DUMMY_GCC_TOOL"
        linker_tool_path = "DUMMY_LINKER_TOOL"
        ar_tool_path = "DUMMY_AR_TOOL"
        strip_tool_path = "DUMMY_STRIP_TOOL"
        for tool in tool_paths:
            if tool.name == "gcc":
                gcc_tool_path = tool.path
                linker_tool_path = _paths_join(ctx.label.workspace_root, ctx.label.package, tool.path)
            elif tool.name == "ar":
                ar_tool_path = tool.path
            elif tool.name == "strip":
                strip_tool_path = tool.path

        legacy_features = []

        # TODO(b/30109612): Remove fragile legacyCompileFlags shuffle once there
        # are no legacy crosstools.
        # Existing projects depend on flags from legacy toolchain fields appearing
        # first on the compile command line. 'legacy_compile_flags' feature contains
        # all these flags, and so it needs to appear before other features.
        if "legacy_compile_flags" in feature_names:
            legacy_compile_flags = ([f for f in features if f.name == "legacy_compile_flags"])[0]
            legacy_features.append(legacy_compile_flags)
        if "default_compile_flags" in feature_names:
            default_compile_flags = ([f for f in features if f.name == "default_compile_flags"])[0]
            legacy_features.append(default_compile_flags)
        platform = "mac" if target_libc == "macosx" else "linux"
        legacy_features.extend(get_legacy_features(
            ctx,
            platform,
            feature_names,
            linker_tool_path,
        ))
        legacy_features.extend([f for f in features if f.name not in ["legacy_compile_flags", "default_compile_flags"]])
        legacy_features.extend(get_features_to_appear_last(feature_names))

        legacy_action_configs = []
        legacy_action_configs.extend(get_legacy_action_configs(
            platform,
            gcc_tool_path,
            ar_tool_path,
            strip_tool_path,
            action_config_names,
        ))
        legacy_action_configs.extend(action_configs)

        features = legacy_features
        action_configs = legacy_action_configs

    return struct(
        _action_configs_DO_NOT_USE = action_configs,
        _artifact_name_patterns_DO_NOT_USE = artifact_name_patterns,
        _features_DO_NOT_USE = features,
        _exec_os_DO_NOT_USE = "LINUX",
        abi_libc_version = abi_libc_version or "",
        abi_version = abi_version or "",
        builtin_sysroot = builtin_sysroot or "",
        compiler = compiler,
        cxx_builtin_include_directories = cxx_builtin_include_directories,
        make_variables = make_variables,
        target_cpu = target_cpu or "",
        target_libc = target_libc or "",
        target_system_name = target_system_name or "",
        tool_paths = tool_paths,
        toolchain_id = toolchain_identifier,
    )
