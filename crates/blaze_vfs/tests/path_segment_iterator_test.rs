//! Port of `src/test/java/com/google/devtools/build/lib/vfs/PathSegmentIteratorTest.java`.

use blaze_vfs::PathSegmentIterator;

fn segment_iterator(normalized_path: &str, drive_str_length: usize) -> Vec<&str> {
    PathSegmentIterator::create(normalized_path, drive_str_length).collect()
}

#[test]
fn empty_path() {
    assert!(segment_iterator("", 0).is_empty());
}

#[test]
fn relative_path() {
    assert_eq!(
        segment_iterator("this/is/a/relative/path", 0),
        vec!["this", "is", "a", "relative", "path"]
    );
}

#[test]
fn root_unix() {
    assert!(segment_iterator("/", 1).is_empty());
}

#[test]
fn root_windows() {
    assert!(segment_iterator("C:/", 3).is_empty());
}

#[test]
fn absolute_path_unix() {
    assert_eq!(
        segment_iterator("/this/is/an/absolute/path", 1),
        vec!["this", "is", "an", "absolute", "path"]
    );
}

#[test]
fn absolute_path_windows() {
    assert_eq!(
        segment_iterator("C:/this/is/an/absolute/path", 3),
        vec!["this", "is", "an", "absolute", "path"]
    );
}

#[test]
fn no_such_element() {
    // Java throws NoSuchElementException; the Rust iterator returns None.
    let mut it = PathSegmentIterator::create("some/path", 0);
    it.next();
    it.next();
    assert_eq!(it.next(), None);
}
