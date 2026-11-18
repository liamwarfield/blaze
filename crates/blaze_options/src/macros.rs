//! The [`options!`] declaration macro: the Rust form of a Java `OptionsBase` subclass with
//! `@Option`-annotated fields. See `TRANSCRIBE.md`.

/// Declares an options class.
///
/// ```ignore
/// blaze_options::options! {
///     /// Options for testing.
///     pub struct TestOptions {
///         #[option(
///             name = "test_string",
///             documentation_category = UNCATEGORIZED,
///             effect_tags = [NO_OP],
///             default_value = "test string default",
///             help = "a string-valued option to test simple option operations",
///         )]
///         pub test_string: String,
///     }
/// }
/// ```
///
/// Every field must carry exactly one `#[option(...)]` attribute, optionally preceded by doc
/// comments and followed by other attributes. Keys are the snake_case names of the Java
/// annotation members: `name`, `abbrev`, `help`, `value_help`, `default_value` (alias
/// `default`), `category`, `documentation_category`, `effect_tags`, `metadata_tags`,
/// `converter`, `allow_multiple`, `expansion`, `implicit_requirements`, `deprecation_warning`,
/// `old_name`, `old_name_warning`, plus `deprecated = true` for a Java `@Deprecated` field.
/// Enum constants (`NO_OP`, `UNDOCUMENTED`, `HIDDEN`, ...) may be written bare.
///
/// A leading `#[uses_only_core_types]` marks a Java `@UsesOnlyCoreTypes` class.
#[macro_export]
macro_rules! options {
    (#[uses_only_core_types] $($rest:tt)*) => {
        $crate::options!(@class true; $($rest)*);
    };
    (@class $core:expr;
        $(#[$smeta:meta])*
        $svis:vis struct $name:ident {
            $(
                $(#[doc = $fdoc:expr])*
                #[option( $($key:ident = $val:expr),* $(,)? )]
                $(#[$fmeta:meta])*
                $fvis:vis $field:ident : $fty:ty
            ),* $(,)?
        }
    ) => {
        $(#[$smeta])*
        #[derive(Clone, Debug)]
        $svis struct $name {
            $(
                $(#[doc = $fdoc])*
                $(#[$fmeta])*
                $fvis $field: $fty,
            )*
        }

        impl $name {
            #[doc(hidden)]
            #[allow(dead_code)]
            fn __new_raw() -> Self {
                $name {
                    $( $field: <$fty as $crate::FieldType>::raw_default(), )*
                }
            }
        }

        impl $crate::OptionsBase for $name {
            fn options_class(&self) -> &'static $crate::OptionsClassInfo {
                <$name as $crate::OptionsClass>::class()
            }
            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn ::core::any::Any {
                self
            }
            fn clone_box(&self) -> ::std::boxed::Box<dyn $crate::OptionsBase> {
                ::std::boxed::Box::new(::core::clone::Clone::clone(self))
            }
        }

        impl $crate::OptionsClass for $name {
            fn class() -> &'static $crate::OptionsClassInfo {
                static INFO: $crate::__private::LazyLock<$crate::OptionsClassInfo> =
                    $crate::__private::LazyLock::new(|| {
                        $crate::OptionsClassInfo::new::<$name>(
                            ::core::stringify!($name),
                            $core,
                            ::std::vec![
                                $(
                                    $crate::OptionDefinition::new::<$name, $fty>(
                                        ::core::stringify!($name),
                                        ::core::stringify!($field),
                                        {
                                            #[allow(unused_mut)]
                                            let mut a = $crate::OptionAnnotation::DEFAULT;
                                            $( $crate::__option_attr!(a, $key, $val); )*
                                            a
                                        },
                                        |o| {
                                            let o = o.downcast_ref::<$name>().unwrap();
                                            <$fty as $crate::FieldType>::to_value(&o.$field)
                                        },
                                        |o, v| {
                                            let o = o.downcast_mut::<$name>().unwrap();
                                            o.$field =
                                                <$fty as $crate::FieldType>::from_value(v.as_ref())?;
                                            ::core::result::Result::Ok(())
                                        },
                                    ),
                                )*
                            ],
                            || ::std::boxed::Box::new($name::__new_raw()),
                        )
                    });
                &INFO
            }
        }

        impl ::core::cmp::PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                <dyn $crate::OptionsBase>::options_equal(self, other)
            }
        }

        impl ::core::default::Default for $name {
            /// `Options.getDefaults(Class)`: an instance with every option at its default value.
            fn default() -> Self {
                $crate::Options::<$name>::get_defaults()
            }
        }
    };
    ($($rest:tt)*) => {
        $crate::options!(@class false; $($rest)*);
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __option_attr {
    ($a:ident, name, $v:expr) => {
        $a.name = $v;
    };
    ($a:ident, abbrev, $v:expr) => {
        $a.abbrev = $v;
    };
    ($a:ident, help, $v:expr) => {
        $a.help = $v;
    };
    ($a:ident, value_help, $v:expr) => {
        $a.value_help = $v;
    };
    ($a:ident, default_value, $v:expr) => {
        $a.default_value = ::core::option::Option::Some($v);
    };
    ($a:ident, default, $v:expr) => {
        $a.default_value = ::core::option::Option::Some($v);
    };
    ($a:ident, category, $v:expr) => {
        $a.category = $v;
    };
    ($a:ident, documentation_category, $v:expr) => {{
        #[allow(unused_imports)]
        use $crate::OptionDocumentationCategory;
        #[allow(unused_imports)]
        use $crate::OptionDocumentationCategory::*;
        $a.documentation_category = ::core::option::Option::Some($v);
    }};
    ($a:ident, effect_tags, $v:expr) => {{
        #[allow(unused_imports)]
        use $crate::OptionEffectTag;
        #[allow(unused_imports)]
        use $crate::OptionEffectTag::*;
        $a.effect_tags = ::core::option::Option::Some(&$v);
    }};
    ($a:ident, metadata_tags, $v:expr) => {{
        #[allow(unused_imports)]
        use $crate::OptionMetadataTag;
        #[allow(unused_imports)]
        use $crate::OptionMetadataTag::*;
        $a.metadata_tags = &$v;
    }};
    ($a:ident, converter, $v:expr) => {
        $a.converter = ::core::option::Option::Some(
            $crate::__private::Arc::new($v) as $crate::__private::Arc<dyn $crate::Converter>
        );
    };
    ($a:ident, allow_multiple, $v:expr) => {
        $a.allow_multiple = $v;
    };
    ($a:ident, expansion, $v:expr) => {
        $a.expansion = &$v;
    };
    ($a:ident, implicit_requirements, $v:expr) => {
        $a.implicit_requirements = &$v;
    };
    ($a:ident, deprecation_warning, $v:expr) => {
        $a.deprecation_warning = $v;
    };
    ($a:ident, old_name, $v:expr) => {
        $a.old_name = $v;
    };
    ($a:ident, old_name_warning, $v:expr) => {
        $a.old_name_warning = $v;
    };
    ($a:ident, deprecated, $v:expr) => {
        $a.deprecated = $v;
    };
}
