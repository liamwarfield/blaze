# Transcribing Java `@Option` classes

Every Java `OptionsBase` subclass becomes one `blaze_options::options!` block. The transcription
is mechanical: copy every annotation member, change only the spelling rules below, never change a
name, default, help string, tag or converter. The worked example at the end is compiled and
tested in `tests/transcribe_example_test.rs`.

## Class

| Java | Rust |
| --- | --- |
| `public class FooOptions extends OptionsBase {` | `options! { pub struct FooOptions {` |
| Javadoc on the class | `///` doc comment before `pub struct` |
| `@UsesOnlyCoreTypes` on the class | `#[uses_only_core_types]` as the very first token inside `options! {` |
| `extends SomeOtherOptions` | copy the parent's `@Option` fields into the struct (the parser only sees fields) and add `// PORT: inherits <Parent>` |
| `FragmentOptions` subclasses | same macro; the fragment-specific methods (`getExec`, ...) go in a normal `impl` block |
| non-`@Option` fields, static constants, helper methods | outside the macro: `pub const` items and a separate `impl FooOptions { ... }`. `options!` structs contain only `@Option` fields |
| `public static final FooOptions DEFAULTS = Options.getDefaults(FooOptions.class)` | `FooOptions::default()` (the macro's `Default` is `Options.getDefaults`) |

Put `// Java: com/google/.../FooOptions.java:<line> FooOptions` above the `options!` block.

## Fields

Each `@Option(...) public T field;` becomes

```rust
#[option(key = value, ...)]
pub snake_case_field: RustType,
```

Field name: Java camelCase to snake_case (PORTING.md "Names"). The option name never changes.
Doc comments on the Java field become `///` lines *before* `#[option(...)]`; other Rust
attributes may follow it.

### Annotation members

Keys are the Java member names in snake_case. Order does not matter; omit members the Java omits.

| Java member | Rust key | Value form |
| --- | --- | --- |
| `name = "x"` | `name = "x"` | string literal or `&'static str` const |
| `abbrev = 'k'` | `abbrev = 'k'` | char |
| `help = "a" + "b"` | `help = "ab"` or `help = concat!("a", "b")` | join the concatenation exactly, keep spaces |
| `valueHelp` | `value_help` | string |
| `defaultValue = "..."` | `default_value = "..."` (alias `default`) | string, exactly as Java (including `"null"`) |
| `category` (deprecated) | `category` | string |
| `documentationCategory = OptionDocumentationCategory.X` | `documentation_category = X` | bare variant |
| `effectTags = {OptionEffectTag.A, OptionEffectTag.B}` | `effect_tags = [A, B]` | array of bare variants |
| `metadataTags = {OptionMetadataTag.HIDDEN}` | `metadata_tags = [HIDDEN]` | array |
| `converter = Foo.class` | `converter = Foo` | a value implementing `Converter` (usually a unit struct); `converter = RangeConverter::new(0, 10)` also works |
| `allowMultiple = true` | `allow_multiple = true` | bool |
| `expansion = {"--a", "--b"}` | `expansion = ["--a", "--b"]` | array of string literals |
| `implicitRequirements = {...}` | `implicit_requirements = [...]` | array of string literals |
| `deprecationWarning` | `deprecation_warning` | string |
| `oldName` | `old_name` | string |
| `oldNameWarning = false` | `old_name_warning = false` | bool |
| `@Deprecated` on the field | `deprecated = true` | bool |

Java string constants used in annotations (`defaultValue = TEST_STRING_DEFAULT`) may be used
directly if transcribed as `pub const X: &str`. A concatenation with a constant
(`"--a=" + CONST`) must be written out as one literal.

Enum constants may be written bare (`NO_OP`) or qualified (`OptionEffectTag::NO_OP`); both
resolve inside the macro.

### Field types

| Java field type | Rust type |
| --- | --- |
| `boolean` | `bool` |
| `int` / `long` / `double` | `i32` / `i64` / `f64` |
| `Integer`, `Long`, `Boolean`, `Double` (nullable) | `Option<i32>` etc. |
| `String` with a non-`"null"` default | `String` |
| `String` with `defaultValue = "null"` | `Option<String>` |
| `List<T>` (`allowMultiple = true`) | `Vec<T>` |
| `List<T>` without `allowMultiple` (list-returning converter) | `Vec<T>`, or `Option<Vec<T>>` if the default is `"null"` |
| `TriState` | `TriState` |
| `java.time.Duration` | `std::time::Duration` |
| `Void` (expansion options) | `Void` |
| `Map.Entry<String, String>` | `(String, String)` |
| `Map.Entry<String, List<String>>` | `(String, Vec<String>)` |
| `java.util.logging.Level` | `LogLevel` |
| `OptionalInt` | `OptionalInt` |
| `RegexPatternOption` | `RegexPatternOption` |
| `Converters.EnvVar` | `EnvVar` |
| `Label`, `PackageIdentifier`, `PathFragment`, `DigestHashFunction` | `Foreign<Label>` etc. (`blaze_options::Foreign`, derefs to the inner value; converters return `Value::new(Foreign(x))`) |
| `Set<E>` of a Java enum (an `EnumSet`, e.g. from `EnumFilterConverter`) | `EnumSet<E>` |
| a Java enum | an `option_enum!` (see below) |
| any other converter result type `X` | a Rust type implementing `OptionScalar`, then `option_field_type!(X, "java.class.Name")` |

`OptionsData` rejects at parser construction (`ConstructionException`) a `"null"` default on a
type that cannot hold null and an `allow_multiple` option whose type is not a `Vec`.

## Converters

| Java | Rust |
| --- | --- |
| `Converters.XConverter` | `blaze_options::XConverter` (same names) |
| `class FooConverter extends EnumConverter<Foo> { FooConverter() { super(Foo.class, "foo"); } }` | `enum_converter!(pub FooConverter, Foo, "foo");` |
| `class FooConverter extends BoolOrEnumConverter<Foo> { ... super(Foo.class, "foo", Foo.YES, Foo.NO) }` | `bool_or_enum_converter!(pub FooConverter, Foo, "foo", Foo::YES, Foo::NO);` |
| `class C extends Converter.Contextless<T>` | `#[derive(Default)] struct C;` + `impl Converter for C` (`convert` returns `Ok(Some(Value::new(t)))`; Java `null` is `Ok(None)`; `List<T>` results use `Value::list_of(...)`) |
| `class C extends RangeConverter { C() { super(0, 100); } }` | unit struct delegating to `RangeConverter::new(0, 100)`, like `PercentageConverter` |

Error text of converters must be copied character for character.

## Enums

```rust
option_enum! {
    /// Enum to select whether color output is enabled or not.
    pub enum UseColor { YES, NO, AUTO }
}
```

Variants keep the Java constant names. If the Java enum overrides `toString()`, give each
variant its string: `pub enum CompilationMode { FASTBUILD = "fastbuild", DBG = "dbg", OPT = "opt" }`.
`EnumConverter` matches against `toString()`, case-insensitively, as in Java. Javadoc on a
constant becomes a `///` comment before the variant. A Java enum that `implements StarlarkValue`
starts with `#[starlark_value]` (before any doc comment); this makes its `EnumConverter` and
`BoolOrEnumConverter` Starlark-convertible:

```rust
option_enum! {
    #[starlark_value]
    /// Values for the --include_config_fragments_provider option.
    pub enum CoreOptionsIncludeConfigFragmentsEnum {
        /// Don't offer the provider at all.
        OFF,
        DIRECT,
        TRANSITIVE,
    }
}
```

## Worked example: `runtime/KeepGoingOption.java` and part of `runtime/UiOptions.java`

Java:

```java
/** Defines the --keep_going option which is used by multiple commands. */
public class KeepGoingOption extends OptionsBase {
  @Option(
    name = "keep_going",
    abbrev = 'k',
    defaultValue = "false",
    documentationCategory = OptionDocumentationCategory.EXECUTION_STRATEGY,
    effectTags = {OptionEffectTag.EAGERNESS_TO_EXIT},
    help =
        "Continue as much as possible after an error.  While the target that failed and those "
            + "that depend on it cannot be analyzed, other prerequisites of these targets can be."
  )
  public boolean keepGoing;
}

public class UiOptions extends OptionsBase {
  /** Enum to select whether color output is enabled or not. */
  public enum UseColor { YES, NO, AUTO }

  /** Converter for {@link UseColor}. */
  public static class UseColorConverter extends EnumConverter<UseColor> {
    public UseColorConverter() { super(UseColor.class, "--color setting"); }
  }

  @Option(
      name = "show_progress_rate_limit",
      defaultValue = "0.2", // A nice middle ground; snappy but not too spammy in logs.
      documentationCategory = OptionDocumentationCategory.UNCATEGORIZED,
      effectTags = {OptionEffectTag.UNKNOWN},
      help = "Minimum number of seconds between progress messages in the output.")
  public double showProgressRateLimit;

  @Option(
      name = "color",
      defaultValue = "auto",
      converter = UseColorConverter.class,
      documentationCategory = OptionDocumentationCategory.UNCATEGORIZED,
      effectTags = {OptionEffectTag.UNKNOWN},
      help = "Use terminal controls to colorize output.")
  public UseColor useColorEnum;

  @Option(
      name = "terminal_columns",
      defaultValue = "80",
      metadataTags = {OptionMetadataTag.HIDDEN},
      documentationCategory = OptionDocumentationCategory.UNDOCUMENTED,
      effectTags = {OptionEffectTag.UNKNOWN},
      help = "A system-generated parameter which specifies the terminal width in columns.")
  public int terminalColumns;

  @Option(
      name = "ui_actions_shown",
      oldName = "experimental_ui_actions_shown",
      defaultValue = "8",
      ...)
  public int uiActionsShown;

  public boolean useColor() {
    return useColorEnum == UseColor.YES || (useColorEnum == UseColor.AUTO && isATty);
  }
}
```

Rust:

```rust
use blaze_options::{enum_converter, option_enum, options};

// Java: com/google/devtools/build/lib/runtime/KeepGoingOption.java:22 KeepGoingOption
options! {
    /// Defines the --keep_going option which is used by multiple commands.
    pub struct KeepGoingOption {
        #[option(
            name = "keep_going",
            abbrev = 'k',
            default_value = "false",
            documentation_category = EXECUTION_STRATEGY,
            effect_tags = [EAGERNESS_TO_EXIT],
            help = "Continue as much as possible after an error.  While the target that failed \
                    and those that depend on it cannot be analyzed, other prerequisites of these \
                    targets can be.",
        )]
        pub keep_going: bool,
    }
}

option_enum! {
    /// Enum to select whether color output is enabled or not.
    pub enum UseColor { YES, NO, AUTO }
}

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:119 UiOptions.UseColorConverter
enum_converter!(pub UseColorConverter, UseColor, "--color setting");

// Java: com/google/devtools/build/lib/runtime/UiOptions.java:35 UiOptions
options! {
    pub struct UiOptions {
        #[option(
            name = "show_progress_rate_limit",
            default_value = "0.2", // A nice middle ground; snappy but not too spammy in logs.
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Minimum number of seconds between progress messages in the output.",
        )]
        pub show_progress_rate_limit: f64,

        #[option(
            name = "color",
            default_value = "auto",
            converter = UseColorConverter,
            documentation_category = UNCATEGORIZED,
            effect_tags = [UNKNOWN],
            help = "Use terminal controls to colorize output.",
        )]
        pub use_color_enum: UseColor,

        #[option(
            name = "terminal_columns",
            default_value = "80",
            metadata_tags = [HIDDEN],
            documentation_category = UNDOCUMENTED,
            effect_tags = [UNKNOWN],
            help = "A system-generated parameter which specifies the terminal width in columns.",
        )]
        pub terminal_columns: i32,
    }
}

impl UiOptions {
    // Methods of the Java class go here, unchanged in behaviour.
}
```

Note on `help`: Rust string continuation `\` at end of line skips the newline *and the leading
whitespace of the next line*, so keep the separating space before the `\` (as above) exactly
where the Java concatenation had it. When in doubt use `concat!("...", "...")`, which mirrors
Java's `+` piece by piece.

## Using the parser

```rust
let mut parser = OptionsParser::builder()
    .options_classes(&[KeepGoingOption::class(), UiOptions::class()])
    .build()?;                       // ConstructionException
parser.parse(&["-k", "--color=yes"])?; // OptionsParsingException, same text as Java
let ui: UiOptions = parser.get_options::<UiOptions>().unwrap();
```
