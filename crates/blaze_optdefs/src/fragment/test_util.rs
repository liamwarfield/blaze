//! Shared checks for the per-class option tests.

use blaze_options::{OptionsBase, OptionsClass, OptionsParser, OptionsParsingException};

/// The option names of `T`, as the parser sees them (alphabetical).
pub fn option_names<T: OptionsClass>() -> Vec<&'static str> {
    T::class()
        .definitions()
        .map(|d| d.get_option_name())
        .collect()
}

/// Asserts that `T` declares exactly `expected` (the `@Option` names of the Java class, in
/// declaration order).
pub fn assert_option_names<T: OptionsClass>(expected: &[&str]) {
    let mut expected: Vec<&str> = expected.to_vec();
    expected.sort();
    assert_eq!(option_names::<T>(), expected);
}

/// Builds a parser over `T` alone.
pub fn parser<T: OptionsClass>() -> OptionsParser {
    OptionsParser::builder()
        .options_classes(&[T::class()])
        .build()
        .unwrap_or_else(|e| panic!("ConstructionException: {:?}", e))
}

/// Parses `args` with a parser over `T` alone.
pub fn parse<T: OptionsClass>(args: &[&str]) -> Result<T, OptionsParsingException> {
    let mut p = parser::<T>();
    p.parse(args)?;
    Ok(p.get_options::<T>().unwrap())
}

/// Every option's default converts, and parsing `--name=<default string>` yields the default
/// value. Internal options, options whose default is the special `"null"`, expansion/Void options and
/// `allowMultiple` options (where an explicit value appends) are only checked for a convertible
/// default. `skip` lists options whose explicit parse is not comparable (documented per class).
pub fn assert_defaults_round_trip<T: OptionsClass>(skip: &[&str]) {
    let defaults = T::default_instance();
    for def in T::class().definitions() {
        let name = def.get_option_name();
        let default_value = def.get_default_value(None);
        let base: &dyn OptionsBase = &defaults;
        assert_eq!(
            base.get_value_from_definition(def),
            default_value,
            "default of --{}",
            name
        );
        if def.is_special_null_default()
            || def.is_void_field()
            || def.is_expansion_option()
            || def.allows_multiple()
            // Internal options (names with spaces) cannot be set on the command line.
            || def
                .get_option_metadata_tags()
                .contains(&blaze_options::OptionMetadataTag::INTERNAL)
            || skip.contains(&name)
        {
            continue;
        }
        let arg = format!("--{}={}", name, def.get_unparsed_default_value());
        let parsed = parse::<T>(&[&arg]).unwrap_or_else(|e| panic!("{}: {}", arg, e));
        let parsed: &dyn OptionsBase = &parsed;
        assert_eq!(
            parsed.get_value_from_definition(def),
            default_value,
            "round trip of {}",
            arg
        );
    }
}

/// `Options.getDefaults(T.class)` through the trait the macro implements.
pub trait DefaultInstance {
    fn default_instance() -> Self;
}

impl<T: OptionsClass> DefaultInstance for T {
    fn default_instance() -> Self {
        blaze_options::Options::<T>::get_defaults()
    }
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/util/OptionsTestCase.java:42 OptionsTestCase.createWithPrefix
/// `createWithPrefix("--abc=", "x", "y")` is `create("--abc=x", "--abc=y")`.
pub fn create_with_prefix<T: OptionsClass>(prefix: &str, args: &[&str]) -> T {
    let args: Vec<String> = args.iter().map(|a| format!("{}{}", prefix, a)).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    parse::<T>(&args).unwrap()
}

fn normalized_keys<T: super::FragmentOptions>(o: &T) -> (String, String) {
    let n = o.get_normalized();
    let n: &dyn OptionsBase = &n;
    (n.cache_key(), n.java_to_string())
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/util/OptionsTestCase.java:57 OptionsTestCase.assertSame
pub fn assert_same<T: super::FragmentOptions>(one: &T, two: &T) {
    // We normalize first, since that is what BuildOptions.checkSum() does.
    let (k1, s1) = normalized_keys(one);
    let (k2, s2) = normalized_keys(two);
    assert_eq!(k1, k2);
    // Also check equality of toString() as that influences the ST-hash computation.
    assert_eq!(s1, s2);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/util/OptionsTestCase.java:68 OptionsTestCase.assertDifferent
pub fn assert_different<T: super::FragmentOptions>(one: &T, two: &T) {
    let (k1, s1) = normalized_keys(one);
    let (k2, s2) = normalized_keys(two);
    assert_ne!(k1, k2);
    assert_ne!(s1, s2);
}
