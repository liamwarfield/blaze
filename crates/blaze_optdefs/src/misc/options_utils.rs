//! The path converters of `lib/util/OptionsUtils.java`, and the option field type that carries a
//! `PathFragment`.

use std::ops::Deref;

use blaze_options::{
    ConversionContext, Converter, OptionScalar, OptionsParsingException, Value,
    converter::non_null, option_field_type,
};
use blaze_vfs::PathFragment;

type ConvResult = Result<Option<Value>, OptionsParsingException>;

/// A `PathFragment` option value.
///
/// PORT: `blaze_options::FieldType`/`OptionScalar` and `blaze_vfs::PathFragment` are both foreign
/// to this crate, so the orphan rule forbids implementing one for the other here. Option fields of
/// Java type `PathFragment` use this newtype instead; it dereferences to the `PathFragment`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OptionPathFragment(pub PathFragment);

impl Deref for OptionPathFragment {
    type Target = PathFragment;
    fn deref(&self) -> &PathFragment {
        &self.0
    }
}

impl OptionScalar for OptionPathFragment {
    // Java: com/google/devtools/build/lib/vfs/PathFragment.java PathFragment.toString
    fn java_to_string(&self) -> String {
        self.0.get_path_string().to_string()
    }
}

option_field_type!(
    OptionPathFragment,
    "com.google.devtools.build.lib.vfs.PathFragment",
    raw = OptionPathFragment(PathFragment::create(""))
);

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:98 OptionsUtils.PathFragmentConverter
/// Converter from String to PathFragment.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathFragmentConverter;

impl PathFragmentConverter {
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:102 OptionsUtils.PathFragmentConverter.convert
    pub fn convert_path(&self, input: &str) -> PathFragment {
        convert_options_path_fragment(input)
    }
}

impl Converter for PathFragmentConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        // checkNotNull(input)
        let input = non_null(input);
        Ok(Some(Value::new(OptionPathFragment(
            self.convert_path(input),
        ))))
    }
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:107 OptionsUtils.PathFragmentConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a path".into()
    }
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:113 OptionsUtils.EmptyToNullPathFragmentConverter
/// Converter from String to PathFragment. If the input is empty returns `null` instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyToNullPathFragmentConverter;

impl Converter for EmptyToNullPathFragmentConverter {
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:117 OptionsUtils.EmptyToNullPathFragmentConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(None);
        }
        Ok(Some(Value::new(OptionPathFragment(
            convert_options_path_fragment(input),
        ))))
    }
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:125 OptionsUtils.EmptyToNullPathFragmentConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a path".into()
    }
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:131 OptionsUtils.AbsolutePathFragmentConverter
/// Converter from String to PathFragment requiring the provided path to be absolute.
#[derive(Clone, Copy, Debug, Default)]
pub struct AbsolutePathFragmentConverter;

impl Converter for AbsolutePathFragmentConverter {
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:134 OptionsUtils.AbsolutePathFragmentConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        let parsed = convert_options_path_fragment(input);
        if !parsed.is_absolute() {
            return Err(OptionsParsingException::new(format!(
                "Not an absolute path: '{}'",
                input
            )));
        }
        Ok(Some(Value::new(OptionPathFragment(parsed))))
    }
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:143 OptionsUtils.AbsolutePathFragmentConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "an absolute path".into()
    }
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:149 OptionsUtils.PathFragmentListConverter
/// Converts from a colon-separated list of strings into a list of PathFragment instances.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathFragmentListConverter;

impl PathFragmentListConverter {
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:153 OptionsUtils.PathFragmentListConverter.convert
    pub fn convert_list(&self, input: &str) -> Vec<PathFragment> {
        let mut result = Vec::new();
        // String.split(":") drops trailing empty strings; empty pieces are skipped anyway.
        for piece in input.split(':') {
            if !piece.is_empty() {
                result.push(convert_options_path_fragment(piece));
            }
        }
        result
    }
}

impl Converter for PathFragmentListConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::list_of(
            self.convert_list(non_null(input))
                .into_iter()
                .map(OptionPathFragment),
        )))
    }
    // Java: com/google/devtools/build/lib/util/OptionsUtils.java:164 OptionsUtils.PathFragmentListConverter.getTypeDescription
    fn get_type_description(&self) -> String {
        "a colon-separated list of paths".into()
    }
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:169 OptionsUtils.convertOptionsPathFragment
fn convert_options_path_fragment(path: &str) -> PathFragment {
    if !path.is_empty() && path.starts_with("~/") {
        // Java: path.replace("~", StandardSystemProperty.USER_HOME.value()), which replaces every
        // '~'.
        return PathFragment::create(&path.replace('~', &user_home()));
    }
    PathFragment::create(path)
}

/// `StandardSystemProperty.USER_HOME.value()`. The JVM's `user.home` comes from
/// `getpwuid(getuid())->pw_dir`, not `$HOME`.
fn user_home() -> String {
    blaze_util::unported!("StandardSystemProperty.USER_HOME")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(c: &dyn Converter, s: &str) -> ConvResult {
        c.convert(Some(s), None)
    }

    #[test]
    fn path_fragment_converter_normalizes() {
        let v = conv(&PathFragmentConverter, "a//b/../c").unwrap().unwrap();
        assert_eq!(v.java_to_string(), "a/c");
    }

    #[test]
    fn empty_to_null_path_fragment_converter() {
        assert!(
            conv(&EmptyToNullPathFragmentConverter, "")
                .unwrap()
                .is_none()
        );
        let v = conv(&EmptyToNullPathFragmentConverter, "/x/y")
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "/x/y");
    }

    #[test]
    fn absolute_path_fragment_converter() {
        let e = conv(&AbsolutePathFragmentConverter, "rel/path").unwrap_err();
        assert_eq!(e.get_message(), "Not an absolute path: 'rel/path'");
        let v = conv(&AbsolutePathFragmentConverter, "/abs")
            .unwrap()
            .unwrap();
        assert_eq!(v.java_to_string(), "/abs");
    }

    // Java tests: src/test/java/com/google/devtools/build/lib/util/OptionsUtilsTest.java

    #[test]
    #[ignore = "needs OptionsUtils.asShellEscapedString (not part of the options transcription)"]
    fn as_string_of_explicit_options() {}

    #[test]
    #[ignore = "needs OptionsUtils.asShellEscapedString (not part of the options transcription)"]
    fn as_string_of_explicit_options_correct_sorting_by_priority() {}

    #[test]
    #[ignore = "needs OptionsUtils.asShellEscapedString (not part of the options transcription)"]
    fn as_string_of_explicit_options_with_booleans() {}

    #[test]
    #[ignore = "needs OptionsUtils.asShellEscapedString (not part of the options transcription)"]
    fn as_string_of_explicit_options_multiple_options_are_multiple_times() {}

    fn convert_list(input: &str) -> Vec<PathFragment> {
        PathFragmentListConverter.convert_list(input)
    }

    fn convert_one(input: &str) -> PathFragment {
        PathFragmentConverter.convert_path(input)
    }

    fn fragment(s: &str) -> PathFragment {
        PathFragment::create(s)
    }

    #[test]
    fn empty_string_yields_empty_list() {
        assert!(convert_list("").is_empty());
    }

    #[test]
    fn lonely_dot_yields_lonely_dot() {
        assert_eq!(convert_list("."), vec![fragment(".")]);
    }

    #[test]
    fn converter_skips_empty_strings() {
        assert_eq!(
            convert_list("foo::bar:"),
            vec![fragment("foo"), fragment("bar")]
        );
    }

    #[test]
    #[ignore = "needs StandardSystemProperty.USER_HOME (the JVM's user.home)"]
    fn multiple_paths() {
        let home = user_home();
        assert_eq!(
            convert_list("~/foo:foo:/bar/baz:.:/tmp/bang"),
            vec![
                fragment(&format!("{}/foo", home)),
                fragment("foo"),
                fragment("/bar/baz"),
                fragment("."),
                fragment("/tmp/bang"),
            ]
        );
    }

    #[test]
    #[ignore = "needs StandardSystemProperty.USER_HOME (the JVM's user.home)"]
    fn single_path() {
        assert_eq!(convert_one("foo"), fragment("foo"));
        assert_eq!(convert_one("foo/bar/baz"), fragment("foo/bar/baz"));
        assert_eq!(
            convert_one("~/foo"),
            fragment(&format!("{}/foo", user_home()))
        );
    }

    #[test]
    fn absolute_path_fragment_converter_converts_absolute_path() {
        for path in ["/", "/dir/file"] {
            let v = conv(&AbsolutePathFragmentConverter, path).unwrap().unwrap();
            assert_eq!(v.get::<OptionPathFragment>().0, PathFragment::create(path));
        }
    }

    #[test]
    fn absolute_path_fragment_converter_fails_for_relative_path() {
        let e = conv(&AbsolutePathFragmentConverter, "relative/path").unwrap_err();
        assert_eq!(e.get_message(), "Not an absolute path: 'relative/path'");
    }
}
