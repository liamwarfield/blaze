//! `TagClass`: the attribute schema of a class of module extension tags.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/TagClass.java. The Starlark value returned by
//! `tag_class()` is `crate::starlark::repository_module::TagClassValue`.

use blaze_packages::attribute::Attribute;
use indexmap::IndexMap;

/// Represents a tag class, which is a "class" of tags that share the same attribute schema.
// Java: com/google/devtools/build/lib/bazel/bzlmod/TagClass.java:28 TagClass
#[derive(Debug)]
pub struct TagClass {
    /// The list of attributes of this tag class.
    pub attributes: Vec<Attribute>,
    /// Documentation about this tag class.
    pub doc: Option<String>,
    /// A mapping from the <em>public</em> name of an attribute to the position of said
    /// attribute in `attributes`.
    pub attribute_indices: IndexMap<String, usize>,
}

impl TagClass {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/TagClass.java:42 TagClass.create
    pub fn create(attributes: Vec<Attribute>, doc: Option<String>) -> TagClass {
        let mut attribute_indices = IndexMap::with_capacity(attributes.len());
        for (i, attribute) in attributes.iter().enumerate() {
            let name = attribute.get_public_name();
            // Java ImmutableMap.Builder.buildOrThrow: duplicate keys throw.
            if attribute_indices.insert(name.clone(), i).is_some() {
                panic!("IllegalArgumentException: Multiple entries with same key: {name}");
            }
        }
        TagClass {
            attributes,
            doc,
            attribute_indices,
        }
    }
}
