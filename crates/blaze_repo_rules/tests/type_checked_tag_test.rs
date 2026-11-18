//! Port of `TypeCheckedTagTest` (src/test/java/com/google/devtools/build/lib/bazel/bzlmod).

use blaze_repo_rules::bzlmod::starlark_bazel_module::SortKey;

fn key(module_index: i32, tag_index: i32) -> SortKey {
    SortKey {
        module_index,
        tag_index,
    }
}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn basic() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn label() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn label_without_default_value() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn string_list_dict_default() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn multiple_attributes_and_defaults() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn mandatory() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn allowed_values() {}

#[test]
#[ignore = "needs blaze_packages Attribute builders for tag classes (attr(..).build()) and a frozen-heap fixture"]
fn unknown_attr() {}

#[test]
fn sort_key() {
    let key_m1_t1 = key(1, 1);
    let key_m2_t1 = key(2, 1);
    let key_m1_t2 = key(1, 2);
    let key_m2_t2 = key(2, 2);

    assert!(key_m1_t1 < key_m2_t1);
    assert!(key_m2_t1 > key_m1_t1);
    assert!(key_m1_t1 < key_m1_t2);
    assert!(key_m1_t2 > key_m1_t1);
    assert!(key_m2_t1 < key_m2_t2);
    assert!(key_m2_t2 > key_m2_t1);
}
