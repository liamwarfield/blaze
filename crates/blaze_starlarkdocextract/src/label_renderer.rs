//! Port of `starlarkdocextract/LabelRenderer.java`.

use blaze_cmdline::{Label, RepositoryMapping};
use blaze_starlark::label::LabelValue;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value, ValueLike};

/// A renderer of labels (and of `repr()`s of Starlark values containing labels) in API
/// documentation.
// Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:29 LabelRenderer
#[derive(Debug, Clone)]
pub struct LabelRenderer {
    repository_mapping: RepositoryMapping,
    main_repo_name: Option<String>,
}

impl LabelRenderer {
    /// A LabelRenderer which always uses `Label.getShorthandDisplayForm` for rendering.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:34 LabelRenderer.DEFAULT
    pub fn default_renderer() -> LabelRenderer {
        LabelRenderer::new(blaze_cmdline::repository_mapping::EMPTY.clone(), None)
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:38 LabelRenderer.<init>
    pub fn new(repository_mapping: RepositoryMapping, main_repo_name: Option<String>) -> Self {
        LabelRenderer {
            repository_mapping,
            main_repo_name,
        }
    }

    /// Renders a label as an unquoted string via `Label.getShorthandDisplayForm`, further adding
    /// an explicit repo component for labels in the main repo if `main_repo_name` was provided.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:48 LabelRenderer.render
    pub fn render(&self, label: &Label) -> String {
        self.render_with(label, /* shorthand= */ true)
    }

    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:54 LabelRenderer.render
    fn render_with(&self, label: &Label, shorthand: bool) -> String {
        let label_string = if shorthand {
            label.get_shorthand_display_form(Some(&self.repository_mapping))
        } else {
            label.get_display_form(Some(&self.repository_mapping))
        };
        match &self.main_repo_name {
            None => label_string,
            Some(_) if label_string.starts_with('@') => label_string,
            Some(main_repo_name) => {
                // label.getShorthandDisplayForm omits the repo name part for labels in the main
                // repo regardless of what repositoryMapping says. Therefore, if we want to rename
                // the main repo in labels in emitted docs, we have to do so manually.
                if shorthand && label_string == format!("//:{main_repo_name}") {
                    // Special case: the shorthand form of "@foo//:foo" is "@foo".
                    return format!("@{main_repo_name}");
                }
                format!("@{main_repo_name}{label_string}")
            }
        }
    }

    /// Renders the `repr()` of a Starlark value as a string, with any embedded label values first
    /// converted to string values via [`Self::render`].
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:76 LabelRenderer.reprWithoutLabelConstructor
    pub fn repr_without_label_constructor(&self, value: Value<'_>) -> String {
        let mut out = String::new();
        repr_with(value, &mut out, &|label, out| {
            out.push_str(&string_repr(&self.render(label)));
        });
        out
    }

    /// Renders the `repr()` of a Starlark value as a string, with the argument to the `Label`
    /// constructor for any embedded label values produced via `Label.getDisplayForm`, further
    /// adding an explicit repo component for labels in the main repo if `main_repo_name` was
    /// provided.
    // Java: com/google/devtools/build/lib/starlarkdocextract/LabelRenderer.java:98 LabelRenderer.repr
    pub fn repr(&self, value: Value<'_>) -> String {
        let mut out = String::new();
        repr_with(value, &mut out, &|label, out| {
            out.push_str("Label(");
            // For consistency with Starlark.repr(label), we use label.getDisplayForm() instead
            // of the shorthand form.
            out.push_str(&string_repr(
                &self.render_with(label, /* shorthand= */ false),
            ));
            out.push(')');
        });
        out
    }
}

/// `Printer.repr` of a string.
fn string_repr(s: &str) -> String {
    Heap::temp(|heap| heap.alloc_str(s).to_value().to_repr())
}

/// `Printer.repr(o)` with `repr` overridden for labels: the overriding printer is used for the
/// elements of the containers Java's `Printer` prints itself.
///
/// PORT: Java's override also reaches the elements of any value whose `repr(Printer)` prints
/// them through the printer (a struct, a depset, ...); here only lists, tuples and dicts are
/// traversed, other values print with their own `repr`.
// Java: net/starlark/java/eval/Printer.java Printer.repr
fn repr_with(value: Value<'_>, out: &mut String, on_label: &dyn Fn(&Label, &mut String)) {
    if let Some(label) = value.downcast_ref::<LabelValue>() {
        on_label(&label.0, out);
    } else if let Some(list) = ListRef::from_value(value) {
        out.push('[');
        for (i, x) in list.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(x, out, on_label);
        }
        out.push(']');
    } else if let Some(tuple) = TupleRef::from_value(value) {
        out.push('(');
        for (i, x) in tuple.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(x, out, on_label);
        }
        if tuple.len() == 1 {
            out.push(',');
        }
        out.push(')');
    } else if let Some(dict) = DictRef::from_value(value) {
        out.push('{');
        for (i, (k, v)) in dict.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            repr_with(k, out, on_label);
            out.push_str(": ");
            repr_with(v, out, on_label);
        }
        out.push('}');
    } else {
        out.push_str(&value.to_repr());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_cmdline::RepositoryName;
    use starlark::values::dict::AllocDict;
    use starlark::values::list::AllocList;

    fn label(s: &str) -> Label {
        Label::parse_canonical(s).unwrap()
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/LabelRendererTest.java:35 LabelRendererTest.defaultRenderer
    #[test]
    fn default_renderer() {
        let main_repo_label = label("//foo:bar");
        let dep_repo_label = label("@dep//foo:baz");
        let renderer = LabelRenderer::default_renderer();
        Heap::temp(|heap| {
            assert_eq!(renderer.render(&main_repo_label), "//foo:bar");
            let v = heap.alloc(LabelValue(main_repo_label.clone()));
            assert_eq!(renderer.repr_without_label_constructor(v), "\"//foo:bar\"");
            assert_eq!(renderer.repr(v), "Label(\"//foo:bar\")");

            assert_eq!(renderer.render(&dep_repo_label), "@@dep//foo:baz");
            let v = heap.alloc(LabelValue(dep_repo_label.clone()));
            assert_eq!(
                renderer.repr_without_label_constructor(v),
                "\"@@dep//foo:baz\""
            );
            assert_eq!(renderer.repr(v), "Label(\"@@dep//foo:baz\")");
        });
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/LabelRendererTest.java:50 LabelRendererTest.mainRepoLabel_withoutMainRepoName
    #[test]
    fn main_repo_label_without_main_repo_name() {
        let l = label("//foo:bar");
        let shorthand_label = label("//foo");
        let renderer = LabelRenderer::new(blaze_cmdline::repository_mapping::EMPTY.clone(), None);
        assert_eq!(renderer.render(&l), "//foo:bar");
        assert_eq!(renderer.render(&shorthand_label), "//foo");
        Heap::temp(|heap| {
            let dict = heap.alloc(AllocDict([(
                heap.alloc(LabelValue(l.clone())),
                heap.alloc(LabelValue(shorthand_label.clone())),
            )]));
            assert_eq!(
                renderer.repr_without_label_constructor(dict),
                "{\"//foo:bar\": \"//foo\"}"
            );
            assert_eq!(
                renderer.repr(dict),
                "{Label(\"//foo:bar\"): Label(\"//foo:foo\")}"
            );
        });
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/LabelRendererTest.java:67 LabelRendererTest.mainRepoLabel_withMainRepoName
    #[test]
    fn main_repo_label_with_main_repo_name() {
        let l = label("//foo:bar");
        let shorthand_label = label("//foo");
        let ultra_shorthand_label = label("//:my_main");
        let renderer = LabelRenderer::new(
            blaze_cmdline::repository_mapping::EMPTY.clone(),
            Some("my_main".to_owned()),
        );
        assert_eq!(renderer.render(&l), "@my_main//foo:bar");
        assert_eq!(renderer.render(&shorthand_label), "@my_main//foo");
        assert_eq!(renderer.render(&ultra_shorthand_label), "@my_main");
        Heap::temp(|heap| {
            let list = heap.alloc(AllocList([
                heap.alloc(LabelValue(l.clone())),
                heap.alloc(LabelValue(shorthand_label.clone())),
                heap.alloc(LabelValue(ultra_shorthand_label.clone())),
            ]));
            assert_eq!(
                renderer.repr_without_label_constructor(list),
                "[\"@my_main//foo:bar\", \"@my_main//foo\", \"@my_main\"]"
            );
            assert_eq!(
                renderer.repr(list),
                "[Label(\"@my_main//foo:bar\"), Label(\"@my_main//foo:foo\"), \
                 Label(\"@my_main//:my_main\")]"
            );
        });
    }

    // Java: src/test/java/com/google/devtools/build/lib/starlarkdocextract/LabelRendererTest.java:89 LabelRendererTest.remappedRepoLabel
    #[test]
    fn remapped_repo_label() {
        let l = label("@canonical//foo:bar");
        let shorthand_label = label("@canonical//foo");
        let repository_mapping = RepositoryMapping::create(
            [("local", RepositoryName::create("canonical").unwrap())],
            blaze_cmdline::repository_name::MAIN.clone(),
        );
        let renderer = LabelRenderer::new(repository_mapping, Some("my_main".to_owned()));
        assert_eq!(renderer.render(&l), "@local//foo:bar");
        assert_eq!(renderer.render(&shorthand_label), "@local//foo");
        Heap::temp(|heap| {
            let list = heap.alloc(AllocList([
                heap.alloc(LabelValue(l.clone())),
                heap.alloc(LabelValue(shorthand_label.clone())),
            ]));
            assert_eq!(
                renderer.repr_without_label_constructor(list),
                "[\"@local//foo:bar\", \"@local//foo\"]"
            );
            assert_eq!(
                renderer.repr(list),
                "[Label(\"@local//foo:bar\"), Label(\"@local//foo:foo\")]"
            );
        });
    }
}
