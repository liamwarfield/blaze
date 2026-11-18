//! `ctx.fragments` (Java `analysis/FragmentCollection.java`,
//! `starlarkbuildapi/FragmentCollectionApi.java`) and the registry through which other crates
//! provide the Starlark values of configuration fragments (`cpp`, `py`, `bazel_py`, `apple`,
//! `java`, `proto`, `platform`, `coverage`, ...).
//!
//! Java fragments are themselves Starlark values. Here a fragment is a Rust
//! `blaze_build_config::fragment::Fragment`; the crate that ports a fragment's Starlark API
//! registers a [`FragmentStarlarkFactory`] for its fragment class with
//! [`register_fragment_starlark_factory`] (the lead calls the owners' registration functions
//! once, before analysis).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use allocative::Allocative;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::fragment::{Fragment, FragmentClass};
use parking_lot::RwLock;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::rule::RuleContext;

/// Creates the Starlark value of one configuration fragment (Java: the `Fragment` object itself,
/// annotated with `@StarlarkBuiltin`).
pub trait FragmentStarlarkFactory: Send + Sync {
    /// The Starlark value of `fragment`, a fragment of `configuration`.
    fn create<'v>(
        &self,
        fragment: &Arc<dyn Fragment>,
        configuration: &Arc<BuildConfigurationValue>,
        heap: Heap<'v>,
    ) -> Value<'v>;
}

/// Factories by fragment class (`Class.getName()`).
static FRAGMENT_FACTORIES: LazyLock<
    RwLock<HashMap<&'static str, Arc<dyn FragmentStarlarkFactory>>>,
> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Registers the Starlark value factory of `fragment_class`. A later registration for the same
/// class replaces the earlier one.
pub fn register_fragment_starlark_factory(
    fragment_class: FragmentClass,
    factory: Arc<dyn FragmentStarlarkFactory>,
) {
    FRAGMENT_FACTORIES
        .write()
        .insert(fragment_class.get_name(), factory);
}

fn fragment_factory(fragment_class: FragmentClass) -> Option<Arc<dyn FragmentStarlarkFactory>> {
    FRAGMENT_FACTORIES
        .read()
        .get(fragment_class.get_name())
        .cloned()
}

/// What `RuleContext.getStarlarkFragment(name)` gives for one Starlark fragment name.
#[derive(Clone)]
enum FragmentLookup {
    /// The fragment (Java `getConfiguration().getFragment(fragmentClass)`; `None` when the
    /// configuration does not contain it, which Java returns as null).
    Legal(FragmentClass, Option<Arc<dyn Fragment>>),
    /// The `IllegalArgumentException` message ("... has to declare ...").
    Illegal(String),
}

impl fmt::Debug for FragmentLookup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FragmentLookup::Legal(class, fragment) => {
                write!(f, "Legal({}, {})", class.get_name(), fragment.is_some())
            }
            FragmentLookup::Illegal(msg) => write!(f, "Illegal({msg:?})"),
        }
    }
}

/// Java `RuleContext.getStarlarkFragment(name)`: `Ok(None)` for a name that is not a Starlark
/// fragment name of the configuration.
// Java: com/google/devtools/build/lib/analysis/RuleContext.java:512 RuleContext.getStarlarkFragment
fn get_starlark_fragment(rule_context: &RuleContext, name: &str) -> Option<FragmentLookup> {
    let configuration = rule_context.get_configuration();
    let fragment_class = configuration.get_starlark_fragment_by_name(name)?;
    if !rule_context.is_legal_fragment(
        fragment_class.get_simple_name(),
        fragment_class.starlark_name,
    ) {
        return Some(FragmentLookup::Illegal(format!(
            "{} has to declare '{}' as a required fragment in order to access it. Please update \
             the 'fragments' argument of the rule definition (for example: fragments = [\"{}\"])",
            rule_context.get_rule().get_rule_class(),
            name,
            name
        )));
    }
    // Java: getConfiguration().getFragment(fragmentClass)
    let fragment = configuration
        .get_fragments()
        .iter()
        .find(|(c, _)| std::ptr::eq(*c, fragment_class))
        .map(|(_, f)| f.clone());
    Some(FragmentLookup::Legal(fragment_class, fragment))
}

/// The value of `ctx.fragments` (type `fragments`).
// Java: com/google/devtools/build/lib/analysis/FragmentCollection.java:25 FragmentCollection
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct FragmentCollection {
    #[allocative(skip)]
    configuration: Arc<BuildConfigurationValue>,
    /// Java `getFieldNames()` (`ruleContext.getStarlarkFragmentNames()`), in the
    /// configuration's order.
    field_names: Vec<String>,
    /// `RuleContext.getStarlarkFragment(name)` of every field name, computed when the
    /// collection is created (Java computes it on access; the result only depends on the
    /// configuration and the rule's fragment policy).
    #[allocative(skip)]
    fragments: Vec<(String, FragmentLookup)>,
}

starlark_simple_value!(FragmentCollection);

impl fmt::Debug for FragmentCollection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FragmentCollection")
            .field("field_names", &self.field_names)
            .finish()
    }
}

/// Java's default `StarlarkValue.repr` (FragmentCollection only overrides `toString`).
// Java: net/starlark/java/eval/StarlarkValue.java:50 StarlarkValue.repr
const FRAGMENT_COLLECTION_REPR: &str =
    "<unknown object com.google.devtools.build.lib.analysis.FragmentCollection>";

impl fmt::Display for FragmentCollection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(FRAGMENT_COLLECTION_REPR)
    }
}

impl FragmentCollection {
    // Java: com/google/devtools/build/lib/analysis/FragmentCollection.java:28 FragmentCollection.<init>
    pub fn new(rule_context: &RuleContext) -> FragmentCollection {
        let configuration = rule_context.get_configuration().clone();
        // Java: com/google/devtools/build/lib/analysis/RuleContext.java:532 RuleContext.getStarlarkFragmentNames
        let field_names: Vec<String> = configuration
            .get_starlark_fragment_names()
            .map(|n| n.to_string())
            .collect();
        let fragments = field_names
            .iter()
            .filter_map(|n| get_starlark_fragment(rule_context, n).map(|f| (n.clone(), f)))
            .collect();
        FragmentCollection {
            configuration,
            field_names,
            fragments,
        }
    }

    /// Java `StarlarkSubrule.SubruleFragmentCollection`: the fragments a subrule declared.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:566 SubruleFragmentCollection.<init>
    pub fn for_subrule(
        rule_context: &RuleContext,
        subrule_name: &str,
        fragments: &indexmap::IndexSet<String>,
    ) -> FragmentCollection {
        let configuration = rule_context.get_configuration().clone();
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:589 SubruleFragmentCollection.getFieldNames
        let field_names: Vec<String> = fragments.iter().cloned().collect();
        let all_names: Vec<String> = configuration
            .get_starlark_fragment_names()
            .map(|n| n.to_string())
            .collect();
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:572 SubruleFragmentCollection.getValue
        let lookups = all_names
            .iter()
            .filter_map(|name| {
                let fragment_class = configuration.get_starlark_fragment_by_name(name)?;
                if !fragments.contains(name) {
                    return Some((
                        name.clone(),
                        FragmentLookup::Illegal(format!(
                            "{subrule_name} has to declare '{name}' as a required fragment in \
                             order to access it. Please update the 'fragments' argument of the \
                             subrule definition (for example: fragments = [\"{name}\"])"
                        )),
                    ));
                }
                let fragment = configuration
                    .get_fragments()
                    .iter()
                    .find(|(c, _)| std::ptr::eq(*c, fragment_class))
                    .map(|(_, f)| f.clone());
                Some((
                    name.clone(),
                    FragmentLookup::Legal(fragment_class, fragment),
                ))
            })
            .collect();
        FragmentCollection {
            configuration,
            field_names,
            fragments: lookups,
        }
    }

    /// Java `FragmentCollectionApi.getErrorMessageForUnknownField`.
    // Java: com/google/devtools/build/lib/starlarkbuildapi/FragmentCollectionApi.java:40 FragmentCollectionApi.getErrorMessageForUnknownField
    pub fn get_error_message_for_unknown_field(&self, name: &str) -> String {
        format!(
            "There is no configuration fragment named '{}'. Available fragments: {}",
            name,
            self.fields_to_string()
        )
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/FragmentCollectionApi.java:47 FragmentCollectionApi.fieldsToString
    fn fields_to_string(&self) -> String {
        format!("'{}'", self.field_names.join("', '"))
    }

    /// Java `getValue(name)` as a Starlark value; `Err` is the EvalException message.
    // Java: com/google/devtools/build/lib/analysis/FragmentCollection.java:39 FragmentCollection.getValue
    pub fn get_value<'v>(&self, name: &str, heap: Heap<'v>) -> Result<Option<Value<'v>>, String> {
        let Some((_, lookup)) = self.fragments.iter().find(|(n, _)| n == name) else {
            return Ok(None);
        };
        match lookup {
            FragmentLookup::Illegal(msg) => Err(msg.clone()),
            FragmentLookup::Legal(_, None) => Ok(None),
            FragmentLookup::Legal(class, Some(fragment)) => match fragment_factory(class) {
                Some(factory) => Ok(Some(factory.create(fragment, &self.configuration, heap))),
                None => blaze_util::unported!(
                    "Fragment Starlark API",
                    "no Starlark value factory registered for fragment {}",
                    class.get_name()
                ),
            },
        }
    }
}

#[starlark_value(type = "fragments")]
impl<'v> StarlarkValue<'v> for FragmentCollection {
    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(FRAGMENT_COLLECTION_REPR);
    }

    // PORT: Java throws an `EvalException` from `getValue` for a fragment the rule did not
    // declare. starlark-rust's `get_attr` cannot fail, so the error is reported through
    // `get_attr_error_message` (vendored patch 0013): `ctx.fragments.cpp` and
    // `getattr(ctx.fragments, "cpp")` fail with Java's text. `getattr(ctx.fragments, "cpp",
    // default)` returns the default where Java throws (oracle scenario
    // scripts/w4/scenarios/ctx-errors, step frag_undeclared_getattr_default), and `hasattr` can
    // only stop loudly (`has_attr` below); both need a fallible attribute hook in the vendored
    // crate (request in the wave report).
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match self.get_value(attribute, heap) {
            Ok(v) => v,
            Err(_) => None,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/RuleContext.java:512 RuleContext.getStarlarkFragment
    // (the EvalException of an undeclared fragment)
    // Java: com/google/devtools/build/lib/starlarkbuildapi/FragmentCollectionApi.java:40 FragmentCollectionApi.getErrorMessageForUnknownField
    fn get_attr_error_message(&self, attribute: &str) -> Option<String> {
        match self.fragments.iter().find(|(n, _)| n == attribute) {
            Some((_, FragmentLookup::Illegal(message))) => Some(message.clone()),
            _ => Some(self.get_error_message_for_unknown_field(attribute)),
        }
    }

    // Java: net/starlark/java/eval/Starlark.java:967 Starlark.hasattr (`getValue(name) != null`)
    fn has_attr(&self, attribute: &str, heap: Heap<'v>) -> bool {
        match self.get_value(attribute, heap) {
            Ok(v) => v.is_some(),
            // Java: `getValue` throws, so `hasattr(ctx.fragments, "cpp")` fails for a fragment
            // the rule did not declare. starlark-rust's `has_attr` cannot fail (see `get_attr`).
            Err(message) => blaze_util::unported!(
                "Starlark.hasattr",
                "EvalException from FragmentCollection.getValue: {}",
                message
            ),
        }
    }

    fn dir_attr(&self) -> Vec<String> {
        self.field_names.clone()
    }
}
