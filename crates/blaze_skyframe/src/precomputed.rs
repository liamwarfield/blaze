//! Port of `PrecomputedValue`: variables computed outside Skyframe and injected into the graph.
//!
//! PORT: Java's class lives in `lib/skyframe`. It is here, in the engine crate, because every
//! crate that declares or reads a variable depends on this one and on no common higher crate;
//! one key type keeps one graph node per variable name (Java interns `PrecomputedValue.Key` by
//! name). The variables themselves are declared next to their Java owners
//! (`blaze_sky_loading::precomputed`, `blaze_bzlmod::skyframe::precomputed`, ...).

use std::any::Any;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::{
    ComputeError, Environment, Evaluator, EvaluatorBuilder, FunctionHermeticity, Interrupted, Key,
    SkyFunctionName,
};

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:22 SkyFunctions.PRECOMPUTED
pub const PRECOMPUTED: SkyFunctionName = SkyFunctionName("PRECOMPUTED");

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:24 SkyFunctions.CLIENT_ENVIRONMENT_VARIABLE
pub const CLIENT_ENVIRONMENT_VARIABLE: SkyFunctionName =
    SkyFunctionName("CLIENT_ENVIRONMENT_VARIABLE");

/// `ClientEnvironmentFunction.Key`: the name of a client environment variable. `Debug` is
/// `CLIENT_ENVIRONMENT_VARIABLE:<name>` (Java `AbstractSkyKey.toString`).
///
/// PORT: Java's class is in `lib/skyframe`; it is in this crate because action execution
/// (blaze_sky_exec) and bzlmod (blaze_bzlmod), which do not depend on each other, request it.
// Java: com/google/devtools/build/lib/skyframe/ClientEnvironmentFunction.java:37 ClientEnvironmentFunction.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ClientEnvironmentKey(pub String);

impl ClientEnvironmentKey {
    // Java: com/google/devtools/build/lib/skyframe/ClientEnvironmentFunction.java:30 ClientEnvironmentFunction.key
    pub fn key(key_string: &str) -> ClientEnvironmentKey {
        ClientEnvironmentKey(key_string.to_string())
    }
}

impl fmt::Debug for ClientEnvironmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", CLIENT_ENVIRONMENT_VARIABLE, self.0)
    }
}

impl Key for ClientEnvironmentKey {
    type Value = EnvironmentVariableValue;
    const FUNCTION: SkyFunctionName = CLIENT_ENVIRONMENT_VARIABLE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:24 SkyFunctions.CLIENT_ENVIRONMENT_VARIABLE (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: `EnvironmentVariableValue` is a record of one nullable string.
    // Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
    fn values_equal(old: &EnvironmentVariableValue, new: &EnvironmentVariableValue) -> bool {
        old == new
    }
}

/// The value of a client environment variable (`None`: unset). Port of
/// `EnvironmentVariableValue`.
///
/// PORT: in this crate for the reason given at [`ClientEnvironmentKey`].
// Java: com/google/devtools/build/lib/skyframe/EnvironmentVariableValue.java:28 EnvironmentVariableValue
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentVariableValue(pub Option<String>);

impl EnvironmentVariableValue {
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

type AnyValue = dyn Any + Send + Sync;

/// A variable's value. Java: `PrecomputedValue` (the `Object value` it wraps).
///
/// PORT: Java's `equals` is the wrapped object's `equals`; the value is type-erased here, so it
/// carries the `PartialEq` of its type, captured when it is made.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:45 PrecomputedValue
#[derive(Clone)]
pub struct PrecomputedValue {
    value: Arc<AnyValue>,
    equals: fn(&AnyValue, &AnyValue) -> bool,
}

/// `value.equals(other.value)` of two values of which the first is a `T`: a value of another
/// type is not equal (Java's `equals` implementations check the class first).
fn equals_as<T: PartialEq + 'static>(value: &AnyValue, other: &AnyValue) -> bool {
    match (value.downcast_ref::<T>(), other.downcast_ref::<T>()) {
        (Some(value), Some(other)) => value == other,
        _ => false,
    }
}

impl PrecomputedValue {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:113 PrecomputedValue.<init>
    pub fn new<T: PartialEq + Send + Sync + 'static>(value: T) -> PrecomputedValue {
        PrecomputedValue {
            value: Arc::new(value),
            equals: equals_as::<T>,
        }
    }

    /// Returns the value of the variable.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:118 PrecomputedValue.get
    pub fn get(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.value
    }
}

impl PartialEq for PrecomputedValue {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:128 PrecomputedValue.equals
    fn eq(&self, other: &PrecomputedValue) -> bool {
        (self.equals)(&*self.value, &*other.value)
    }
}

impl fmt::Debug for PrecomputedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: Java prints "<BuildVariable " + value + ">"; the value is type-erased here.
        f.write_str("<BuildVariable>")
    }
}

/// `SkyKey` for a [`PrecomputedValue`]: the variable name. `Debug` prints `PRECOMPUTED:<name>`
/// (Java `AbstractSkyKey.toString`).
///
/// PORT: Java's `UnshareableKey` (remote caching of values) is not represented.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:191 PrecomputedValue.Key
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrecomputedKey(pub &'static str);

impl fmt::Debug for PrecomputedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", PRECOMPUTED, self.0)
    }
}

impl Key for PrecomputedKey {
    type Value = PrecomputedValue;
    const FUNCTION: SkyFunctionName = PRECOMPUTED;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:22 SkyFunctions.PRECOMPUTED (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:128 PrecomputedValue.equals
    fn values_equal(old: &PrecomputedValue, new: &PrecomputedValue) -> bool {
        old == new
    }
}

/// A helper object corresponding to a variable in Skyframe. Instances have no internal state.
///
/// PORT: Java's `shareable` flag (remote caching of values) is not represented.
// Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:145 PrecomputedValue.Precomputed
pub struct Precomputed<T> {
    key: PrecomputedKey,
    _value: PhantomData<fn() -> T>,
}

impl<T: PartialEq + Send + Sync + 'static> Precomputed<T> {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:148 Precomputed.<init>
    pub const fn new(key: &'static str) -> Precomputed<T> {
        Precomputed {
            key: PrecomputedKey(key),
            _value: PhantomData,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:156 Precomputed.getKey
    pub fn get_key(&self) -> PrecomputedKey {
        self.key
    }

    /// Retrieves the value of this variable from Skyframe (`None`: unavailable). If the value
    /// was not set, the evaluator fails (no PRECOMPUTED function is registered).
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:167 Precomputed.get
    pub fn get(&self, env: &mut Environment<'_>) -> Result<Option<Arc<T>>, Interrupted> {
        let Some(value) = env.get_value(self.key)? else {
            return Ok(None);
        };
        Ok(Some(self.downcast(&value)))
    }

    /// `Objects.requireNonNull(P.get(env))`, and Java's unboxing of a `Precomputed<Boolean>`:
    /// the variable must be injected.
    pub fn get_required(&self, env: &mut Environment<'_>) -> Result<Arc<T>, ComputeError> {
        match self.get(env)? {
            Some(v) => Ok(v),
            None => panic!("NullPointerException: precomputed value {:?}", self.key),
        }
    }

    fn downcast(&self, value: &PrecomputedValue) -> Arc<T> {
        value.value.clone().downcast::<T>().unwrap_or_else(|_| {
            panic!(
                "ClassCastException: precomputed value {:?} is not a {}",
                self.key,
                std::any::type_name::<T>()
            )
        })
    }

    /// Injects a value into an evaluator being built.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:176 Precomputed.set
    pub fn set(&self, builder: EvaluatorBuilder, value: T) -> EvaluatorBuilder {
        builder.inject(self.key, PrecomputedValue::new(value))
    }

    /// Injects a value between evaluations.
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:176 Precomputed.set
    pub fn set_on(&self, evaluator: &Evaluator, value: T) {
        evaluator.inject(self.key, PrecomputedValue::new(value));
    }
}

impl<T> fmt::Debug for Precomputed<T> {
    // Java: com/google/devtools/build/lib/skyframe/PrecomputedValue.java:181 Precomputed.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Precomputed{{key={:?}, shareable=true}}", self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_values_of_one_type_are_equal() {
        assert_eq!(
            PrecomputedValue::new(String::from("a")),
            PrecomputedValue::new(String::from("a"))
        );
        assert_ne!(
            PrecomputedValue::new(String::from("a")),
            PrecomputedValue::new(String::from("b"))
        );
    }

    #[test]
    fn values_of_different_types_are_not_equal() {
        assert_ne!(PrecomputedValue::new(1u32), PrecomputedValue::new(1u64));
        assert_ne!(PrecomputedValue::new(1u64), PrecomputedValue::new(1u32));
    }

    #[test]
    fn key_to_string() {
        assert_eq!(
            format!("{:?}", PrecomputedKey("starlark_semantics")),
            "PRECOMPUTED:starlark_semantics"
        );
        assert_eq!(
            format!("{:?}", Precomputed::<bool>::new("x")),
            "Precomputed{key=PRECOMPUTED:x, shareable=true}"
        );
    }

    /// Java: `AbstractInMemoryMemoizingEvaluator.pruneInjectedValues`: re-injecting an equal
    /// variable leaves its readers clean; an unequal one re-runs them.
    #[test]
    fn injecting_an_equal_value_does_not_dirty_readers() {
        static VAR: Precomputed<String> = Precomputed::new("var");
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        struct Reader;
        impl Key for Reader {
            type Value = String;
            const FUNCTION: SkyFunctionName = SkyFunctionName("READER");
            const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
        }
        let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let runs2 = runs.clone();
        let ev = VAR
            .set(EvaluatorBuilder::new(), "a".to_string())
            .typed_function::<Reader, _>(move |_, env| {
                runs2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(VAR.get(env)?.map(|v| (*v).clone()))
            })
            .build();
        let ctx = crate::EvaluationContext {
            keep_going: false,
            parallelism: 1,
            ..crate::EvaluationContext::default()
        };
        let eval = |ev: &Evaluator| {
            ev.evaluate(&[crate::SkyKey::new(Reader)], &ctx)
                .unwrap()
                .get(&Reader)
                .map(|v| (*v).clone())
        };
        assert_eq!(eval(&ev).as_deref(), Some("a"));
        VAR.set_on(&ev, "a".to_string());
        assert_eq!(eval(&ev).as_deref(), Some("a"));
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 1);
        VAR.set_on(&ev, "b".to_string());
        assert_eq!(eval(&ev).as_deref(), Some("b"));
        assert_eq!(runs.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
