//! The `PathFragment` converters of `com/google/devtools/build/lib/util/OptionsUtils.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:169 OptionsUtils.convertOptionsPathFragment
fn convert_options_path_fragment(path: &str) -> PathFragment {
    if !path.is_empty() && path.starts_with("~/") {
        // PORT: Java reads the `user.home` system property (the passwd entry of the user); Rust's
        // `home_dir` prefers `$HOME`, which only differs when `$HOME` was overridden.
        #[allow(deprecated)]
        let home = std::env::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| unported!("OptionsUtils.convertOptionsPathFragment without a home"));
        // Java's String.replace replaces every occurrence.
        return PathFragment::create(&path.replace('~', &home));
    }
    PathFragment::create(path)
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:99 OptionsUtils.PathFragmentConverter
/// Converter from String to PathFragment.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathFragmentConverter;

impl Converter for PathFragmentConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        Ok(Some(Value::new(Foreign(convert_options_path_fragment(
            non_null(input),
        )))))
    }
    fn get_type_description(&self) -> String {
        "a path".into()
    }
}

// Java: com/google/devtools/build/lib/util/OptionsUtils.java:113 OptionsUtils.EmptyToNullPathFragmentConverter
/// Converter from String to PathFragment. If the input is empty returns `null` instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmptyToNullPathFragmentConverter;

impl Converter for EmptyToNullPathFragmentConverter {
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        if input.is_empty() {
            return Ok(None);
        }
        Ok(Some(Value::new(Foreign(convert_options_path_fragment(
            input,
        )))))
    }
    fn get_type_description(&self) -> String {
        "a path".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_to_null() {
        assert!(
            EmptyToNullPathFragmentConverter
                .convert(Some(""), None)
                .unwrap()
                .is_none()
        );
        let v = PathFragmentConverter.convert(Some("a//b/"), None).unwrap();
        assert_eq!(v.unwrap().java_to_string(), "a/b");
    }
}
