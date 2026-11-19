//! Port of `packages/Aspect.java`.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::LazyLock;

use parking_lot::Mutex;

use super::AspectClass;
use super::AspectDefinition;
use super::AspectDescriptor;
use super::AspectParameters;
use super::NativeAspectClass;
use super::StarlarkAspectClass;
use crate::attribute::Attribute;
use crate::attribute::dependency_filter::AttributeInfoProvider;

/// The aspect definition is a function of the aspect class + its parameters, so we can cache
/// that. The native aspects are loaded with blaze and are not stateful (Java
/// `Aspect.definitionCache`, keyed by the native class; here by its name, the class identity).
// Java: com/google/devtools/build/lib/packages/Aspect.java:50 Aspect.definitionCache
static DEFINITION_CACHE: LazyLock<
    Mutex<HashMap<String, HashMap<AspectParameters, Arc<AspectDefinition>>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// An instance of a given [`AspectClass`] with loaded definition and parameters (Java
/// `Aspect`): the aspect equivalent of a `Rule`.
///
/// Equality is Java's: the descriptors are equal and the definitions are the same object
/// (Java's `AspectDefinition` does not override `equals`). Definitions are cached per class and
/// parameters, so equal aspects share their definition. Use [`AspectDescriptor`] in keys.
///
/// PORT: Java interns aspects; equality does not depend on it here.
// Java: com/google/devtools/build/lib/packages/Aspect.java:42 Aspect
#[derive(Clone)]
pub struct Aspect {
    aspect_descriptor: AspectDescriptor,
    aspect_definition: Arc<AspectDefinition>,
}

impl Aspect {
    // Java: com/google/devtools/build/lib/packages/Aspect.java:68 Aspect.forNative
    pub fn for_native(
        native_aspect_class: &Arc<dyn NativeAspectClass>,
        parameters: AspectParameters,
    ) -> Aspect {
        let name = native_aspect_class.get_name().to_owned();
        let cached = DEFINITION_CACHE
            .lock()
            .get(&name)
            .and_then(|m| m.get(&parameters))
            .cloned();
        let definition = match cached {
            Some(d) => d,
            None => {
                // PORT: the lock is not held while the definition is built (a native
                // definition may build other aspects); the first stored value wins, so the
                // definition identity stays unique per class and parameters.
                let built = native_aspect_class.get_definition(&parameters);
                DEFINITION_CACHE
                    .lock()
                    .entry(name)
                    .or_default()
                    .entry(parameters.clone())
                    .or_insert(built)
                    .clone()
            }
        };
        Aspect::create_interned(
            AspectClass::Native(native_aspect_class.clone()),
            definition,
            parameters,
        )
    }

    /// Java `forNative(NativeAspectClass)`: with [`AspectParameters::empty`].
    // Java: com/google/devtools/build/lib/packages/Aspect.java:73 Aspect.forNative
    pub fn for_native_without_parameters(
        native_aspect_class: &Arc<dyn NativeAspectClass>,
    ) -> Aspect {
        Aspect::for_native(native_aspect_class, AspectParameters::empty())
    }

    // Java: com/google/devtools/build/lib/packages/Aspect.java:77 Aspect.forStarlark
    pub fn for_starlark(
        starlark_aspect_class: StarlarkAspectClass,
        aspect_definition: Arc<AspectDefinition>,
        parameters: AspectParameters,
    ) -> Aspect {
        Aspect::create_interned(
            AspectClass::Starlark(starlark_aspect_class),
            aspect_definition,
            parameters,
        )
    }

    // Java: com/google/devtools/build/lib/packages/Aspect.java:84 Aspect.createInterned
    fn create_interned(
        aspect_class: AspectClass,
        definition: Arc<AspectDefinition>,
        parameters: AspectParameters,
    ) -> Aspect {
        Aspect {
            aspect_descriptor: AspectDescriptor::of(aspect_class, parameters),
            aspect_definition: definition,
        }
    }

    /// Returns the aspect class required for building the aspect.
    // Java: com/google/devtools/build/lib/packages/Aspect.java:90 Aspect.getAspectClass
    pub fn get_aspect_class(&self) -> &AspectClass {
        self.aspect_descriptor.get_aspect_class()
    }

    /// Returns parameters for evaluation of the aspect.
    // Java: com/google/devtools/build/lib/packages/Aspect.java:95 Aspect.getParameters
    pub fn get_parameters(&self) -> &AspectParameters {
        self.aspect_descriptor.get_parameters()
    }

    // Java: com/google/devtools/build/lib/packages/Aspect.java:99 Aspect.getDescriptor
    pub fn get_descriptor(&self) -> &AspectDescriptor {
        &self.aspect_descriptor
    }

    // Java: com/google/devtools/build/lib/packages/Aspect.java:103 Aspect.getDefinition
    pub fn get_definition(&self) -> &Arc<AspectDefinition> {
        &self.aspect_definition
    }

    /// All aspect attributes are implicit.
    // Java: com/google/devtools/build/lib/packages/Aspect.java:108 Aspect.isAttributeValueExplicitlySpecified
    pub fn is_attribute_value_explicitly_specified(&self, _attribute: &Attribute) -> bool {
        false
    }
}

impl AttributeInfoProvider for Aspect {
    // Java: com/google/devtools/build/lib/packages/Aspect.java:108 Aspect.isAttributeValueExplicitlySpecified
    fn is_attribute_value_explicitly_specified_for(&self, attribute: &Attribute) -> bool {
        self.is_attribute_value_explicitly_specified(attribute)
    }
}

impl PartialEq for Aspect {
    // Java: com/google/devtools/build/lib/packages/Aspect.java:124 Aspect.equals
    fn eq(&self, other: &Aspect) -> bool {
        self.aspect_descriptor == other.aspect_descriptor
            && Arc::ptr_eq(&self.aspect_definition, &other.aspect_definition)
    }
}

impl Eq for Aspect {}

impl Hash for Aspect {
    // Java: com/google/devtools/build/lib/packages/Aspect.java:119 Aspect.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.aspect_descriptor.hash(state);
        std::ptr::hash(Arc::as_ptr(&self.aspect_definition), state);
    }
}

impl fmt::Display for Aspect {
    // Java: com/google/devtools/build/lib/packages/Aspect.java:114 Aspect.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Aspect {}", self.aspect_descriptor)
    }
}

impl fmt::Debug for Aspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
