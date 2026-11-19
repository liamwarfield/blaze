"""Dormant dependencies and a materializer attribute (--experimental_dormant_deps)."""

def _impl(*args):
    fail("should not be called")

r = rule(
    implementation = _impl,
    dependency_resolution_rule = True,
    attrs = {"dormant": attr.dormant_label(), "dormant_list": attr.dormant_label_list()},
)

def _mat(ctx):
    return []

def _consumer_impl(ctx):
    return []

consumer = rule(
    implementation = _consumer_impl,
    attrs = {
        "_materialized": attr.label_list(materializer = _mat),
    },
)
