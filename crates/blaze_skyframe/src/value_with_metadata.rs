//! Port of `ValueWithMetadata`, and the `Reportable` element type of its events.

use crate::{ErrorInfo, SkyKey, SkyValue};
use blaze_collect::{NestedSet, Order};
use blaze_events::{Event, Postable, Reportable};
use blaze_util::check_state;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Java's `Reportable` as stored by a node: an `Event` or a `Postable`.
///
/// Equality: an `Event` by value (Java `Event.equals`); a `Postable` by identity.
/// PORT: a Java `Postable` that overrides `equals` would compare by value; Rust `Postable`s have
/// no equality, so identity (`Arc::ptr_eq`, pointer hash) is used for every one of them.
// Java: com/google/devtools/build/lib/events/Reportable.java:20 Reportable
#[derive(Clone)]
pub(crate) enum StoredReportable {
    Event(Event),
    Post(Arc<dyn Postable>),
}

impl StoredReportable {
    // Java: com/google/devtools/build/lib/events/Reportable.java:54 Reportable.storeForReplay
    pub(crate) fn store_for_replay(&self) -> bool {
        match self {
            StoredReportable::Event(e) => e.store_for_replay(),
            StoredReportable::Post(p) => p.store_for_replay(),
        }
    }
}

impl PartialEq for StoredReportable {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (StoredReportable::Event(a), StoredReportable::Event(b)) => a == b,
            (StoredReportable::Post(a), StoredReportable::Post(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for StoredReportable {}

impl Hash for StoredReportable {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            StoredReportable::Event(e) => {
                0u8.hash(state);
                e.hash(state);
            }
            StoredReportable::Post(p) => {
                1u8.hash(state);
                (Arc::as_ptr(p) as *const () as usize).hash(state);
            }
        }
    }
}

impl fmt::Debug for StoredReportable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoredReportable::Event(e) => fmt::Debug::fmt(e, f),
            StoredReportable::Post(p) => fmt::Debug::fmt(p, f),
        }
    }
}

/// Java: `ValueWithMetadata.NO_EVENTS`.
///
/// PORT: a function returning a clone of one shared empty set (Java's constant); all empty sets
/// are equal under `shallow_equals`, which is what `ValueWithEvents.equals` observes.
// Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:29 ValueWithMetadata.NO_EVENTS
pub(crate) fn no_events() -> NestedSet<StoredReportable> {
    static NO_EVENTS: std::sync::LazyLock<NestedSet<StoredReportable>> =
        std::sync::LazyLock::new(|| Order::STABLE_ORDER.empty_set());
    NO_EVENTS.clone()
}

/// Encapsulation of data stored by `NodeEntry` when the value has finished building.
///
/// PORT: in Java a node stores a `SkyValue` that is either the bare value (when there is no
/// error and no event: `normal` returns the value itself "in order to save memory") or a
/// `ValueWithMetadata` subclass. Rust has no `instanceof` on `SkyValue`, so the bare case is the
/// [`ValueWithMetadata::Bare`] variant of this one type; the static helpers that take a
/// "maybe-wrapped" `SkyValue` in Java (`justValue`, `getMaybeErrorInfo`, `getEvents`,
/// `wrapWithMetadata`) take this type.
///
/// PORT: Java's `NotComparableValueWithEvents` (a `ValueWithEvents` whose value is a
/// `NotComparableSkyValue`) has no separate variant: whether a value is comparable is decided by
/// its key ([`crate::Key::values_equal`]), which `equals` consults.
// Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:26 ValueWithMetadata
#[derive(Clone)]
pub(crate) enum ValueWithMetadata {
    /// A value with no error and no events, stored bare.
    Bare(SkyValue),
    /// Implementation of `ValueWithMetadata` for the value case.
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:76 ValueWithMetadata.ValueWithEvents
    ValueWithEvents {
        value: SkyValue,
        transitive_events: NestedSet<StoredReportable>,
    },
    /// Implementation of `ValueWithMetadata` for the error case.
    ///
    /// Mark NotComparableSkyValue because it's unlikely that re-evaluation gives the same error.
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:152 ValueWithMetadata.ErrorInfoValue
    ErrorInfoValue {
        error_info: Arc<ErrorInfo>,
        value: Option<SkyValue>,
        transitive_events: NestedSet<StoredReportable>,
    },
}

impl ValueWithMetadata {
    /// Builds a value entry that has an error (and no value).
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:37 ValueWithMetadata.error
    pub(crate) fn error(
        error_info: ErrorInfo,
        transitive_events: NestedSet<StoredReportable>,
    ) -> ValueWithMetadata {
        ValueWithMetadata::normal(None, Some(error_info), transitive_events)
    }

    /// Builds a SkyValue that has a value, and possibly an error, and possibly events/postables.
    /// If it has only a value, returns just the value in order to save memory.
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:46 ValueWithMetadata.normal
    pub(crate) fn normal(
        value: Option<SkyValue>,
        error_info: Option<ErrorInfo>,
        transitive_events: NestedSet<StoredReportable>,
    ) -> ValueWithMetadata {
        check_state!(
            value.is_some() || error_info.is_some(),
            "Value and error cannot both be null"
        );
        let Some(error_info) = error_info else {
            let value = value.expect("checked above");
            return if transitive_events.is_empty() {
                ValueWithMetadata::Bare(value)
            } else {
                ValueWithMetadata::create_value_with_events(value, transitive_events)
            };
        };
        ValueWithMetadata::ErrorInfoValue {
            error_info: Arc::new(error_info),
            value,
            transitive_events,
        }
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:84 ValueWithEvents.createValueWithEvents
    // PORT: the `NotComparableValueWithEvents` branch is decided by the key at comparison time
    // (see the type's doc).
    fn create_value_with_events(
        value: SkyValue,
        transitive_events: NestedSet<StoredReportable>,
    ) -> ValueWithMetadata {
        ValueWithMetadata::ValueWithEvents {
            value,
            transitive_events,
        }
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:60 ValueWithMetadata.hasError
    #[allow(dead_code)] // ported API, used by tests
    pub(crate) fn has_error(&self) -> bool {
        matches!(self, ValueWithMetadata::ErrorInfoValue { .. })
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:65 ValueWithMetadata.getValue
    pub(crate) fn get_value(&self) -> Option<&SkyValue> {
        match self {
            ValueWithMetadata::Bare(value) => Some(value),
            ValueWithMetadata::ValueWithEvents { value, .. } => Some(value),
            ValueWithMetadata::ErrorInfoValue { value, .. } => value.as_ref(),
        }
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:70 ValueWithMetadata.getErrorInfo
    pub(crate) fn get_error_info(&self) -> Option<&ErrorInfo> {
        match self {
            ValueWithMetadata::ErrorInfoValue { error_info, .. } => Some(error_info),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:72 ValueWithMetadata.getTransitiveEvents
    pub(crate) fn get_transitive_events(&self) -> NestedSet<StoredReportable> {
        match self {
            // Java: wrapWithMetadata(bare) has NO_EVENTS.
            ValueWithMetadata::Bare(_) => no_events(),
            ValueWithMetadata::ValueWithEvents {
                transitive_events, ..
            }
            | ValueWithMetadata::ErrorInfoValue {
                transitive_events, ..
            } => transitive_events.clone(),
        }
    }

    /// Java `this.equals(o)` for the stored values of the node of `key`.
    ///
    /// `ValueWithEvents`: we override equals so that if the same value is written to a
    /// `NodeEntry` twice, it can verify that the two values are equal, and avoid incrementing its
    /// version.
    ///
    /// PORT: Java's identity shortcut (`this == o`) is dropped: Rust cannot tell a
    /// `NotComparableSkyValue` from its type, and a node never compares a stored value with
    /// itself (`setValue` receives a value built for this commit). A bare value's `equals` is
    /// the key's [`crate::Key::values_equal`]; it is never equal to a `ValueWithMetadata`.
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:109 ValueWithEvents.equals
    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:182 ErrorInfoValue.equals
    pub(crate) fn equals(&self, key: &SkyKey, o: &ValueWithMetadata) -> bool {
        match (self, o) {
            (ValueWithMetadata::Bare(a), ValueWithMetadata::Bare(b)) => {
                key.argument().dyn_values_equal(a, b)
            }
            (
                ValueWithMetadata::ValueWithEvents {
                    value,
                    transitive_events,
                },
                ValueWithMetadata::ValueWithEvents {
                    value: that_value,
                    transitive_events: that_transitive_events,
                },
            ) => {
                // Shallow equals is a middle ground between using default equals, which might
                // miss nested sets with the same elements, and deep equality checking, which
                // would be expensive. All three choices are sound, since shallow equals and
                // default equals are more conservative than deep equals. Using shallow equals
                // means that we may unnecessarily consider some values unequal that are actually
                // equal, but this is still a net win over deep equals.
                key.argument().dyn_values_equal(value, that_value)
                    && transitive_events.shallow_equals(Some(that_transitive_events))
            }
            (
                ValueWithMetadata::ErrorInfoValue { .. },
                ValueWithMetadata::ErrorInfoValue { .. },
            ) => {
                // Objects.equals(errorInfo, that.errorInfo): ErrorInfo.equals has no port.
                blaze_util::unported!("ErrorInfoValue.equals")
            }
            _ => false,
        }
    }

    /// Whether Java's stored object implements `NotComparableSkyValue` as far as the wrapper
    /// tells: `ErrorInfoValue` does. A bare or `ValueWithEvents` value's comparability is its
    /// key's (see the type's doc).
    pub(crate) fn is_error_info_value(&self) -> bool {
        matches!(self, ValueWithMetadata::ErrorInfoValue { .. })
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:225 ValueWithMetadata.justValue
    pub(crate) fn just_value(value: &ValueWithMetadata) -> Option<SkyValue> {
        value.get_value().cloned()
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:232 ValueWithMetadata.wrapWithMetadata
    #[allow(dead_code)] // ported API, used by tests
    pub(crate) fn wrap_with_metadata(value: &ValueWithMetadata) -> ValueWithMetadata {
        match value {
            ValueWithMetadata::Bare(v) => {
                ValueWithMetadata::create_value_with_events(v.clone(), no_events())
            }
            other => other.clone(),
        }
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:240 ValueWithMetadata.getMaybeErrorInfo
    pub(crate) fn get_maybe_error_info(value: &ValueWithMetadata) -> Option<&ErrorInfo> {
        value.get_error_info()
    }

    // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:247 ValueWithMetadata.getEvents
    pub(crate) fn get_events(value: &ValueWithMetadata) -> NestedSet<StoredReportable> {
        value.get_transitive_events()
    }
}

impl fmt::Debug for ValueWithMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // PORT: a Rust SkyValue (`dyn Any`) cannot be printed; `<SkyValue>` stands for Java's
        // `value.toString()`.
        match self {
            ValueWithMetadata::Bare(_) => f.write_str("<SkyValue>"),
            // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:132 ValueWithEvents.toString
            ValueWithMetadata::ValueWithEvents {
                transitive_events, ..
            } => write!(
                f,
                "ValueWithEvents{{value=<SkyValue>, transitiveEvents size={}}}",
                transitive_events.memoized_flatten_and_get_size()
            ),
            // Java: com/google/devtools/build/skyframe/ValueWithMetadata.java:209 ErrorInfoValue.toString
            ValueWithMetadata::ErrorInfoValue {
                error_info, value, ..
            } => {
                let mut result = String::new();
                if value.is_some() {
                    result.push_str("Value: <SkyValue>");
                }
                if !result.is_empty() {
                    result.push_str("; ");
                }
                result.push_str(&format!("Error: {error_info:?}"));
                f.write_str(&result)
            }
        }
    }
}
