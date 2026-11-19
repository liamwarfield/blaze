"""Rules of the scripts/exec2 probe package."""

def _pruning_rule_impl(ctx):
    inputs = ctx.attr.inputs.files
    output = ctx.outputs.out
    unused_inputs_list = ctx.actions.declare_file(ctx.label.name + ".unused")
    arguments = [output.path, unused_inputs_list.path]
    for input in inputs.to_list():
        arguments.append(input.path)
    ctx.actions.run(
        inputs = inputs,
        outputs = [output, unused_inputs_list],
        arguments = arguments,
        executable = ctx.file.tool,
        unused_inputs_list = unused_inputs_list,
        mnemonic = "PruneProbe",
    )

pruning_rule = rule(
    implementation = _pruning_rule_impl,
    attrs = {
        "inputs": attr.label(),
        "tool": attr.label(allow_single_file = True, cfg = "exec"),
        "out": attr.output(),
    },
)

def _tree_sources_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name + "_dir")
    ctx.actions.run_shell(
        inputs = [ctx.file.template],
        outputs = [out],
        command = """
set -eu
d="$1"
v="$(cat "$2")"
mkdir -p "$d/sub"
cat > "$d/util.h" <<EOT
#pragma once
int util_value();
EOT
cat > "$d/a.cc" <<EOT
#include "util.h"
int util_value() { return $v; }
EOT
cat > "$d/sub/b.cc" <<EOT
int b_value() { return $v + 1; }
EOT
cat > "$d/c.c" <<EOT
int c_value(void) { return 3; }
EOT
""",
        arguments = [out.path, ctx.file.template.path],
        mnemonic = "GenTree",
    )
    return [DefaultInfo(files = depset([out]))]

tree_sources = rule(
    implementation = _tree_sources_impl,
    attrs = {"template": attr.label(allow_single_file = True)},
)
