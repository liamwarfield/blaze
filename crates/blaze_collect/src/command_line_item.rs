//! Port of `com.google.devtools.build.lib.actions.CommandLineItem` and
//! `CommandLineExpansionException`.
// PORT: these live in `actions`, but Java's `nestedset:fingerprint_cache` target depends on
// `actions:commandline_item`, while the actions crate depends on this crate. To avoid a crate
// cycle they are ported here; the actions crate should re-export them.

use std::any::Any;
use std::fmt;

/// Something that can be expanded to a command line string.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:19 CommandLineItem
pub trait CommandLineItem {
    // Java: com/google/devtools/build/lib/actions/CommandLineItem.java:67 CommandLineItem.expandToCommandLine
    fn expand_to_command_line(&self) -> String;
}

// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:75 CommandLineItem.expandToCommandLine(Object)
// PORT: Java falls back to `object.toString()` for values that are not CommandLineItems. Rust has
// no such fallback, so types opt in; String-like and integer types expand to their `toString`.
impl CommandLineItem for String {
    fn expand_to_command_line(&self) -> String {
        self.clone()
    }
}

impl CommandLineItem for str {
    fn expand_to_command_line(&self) -> String {
        self.to_string()
    }
}

impl<T: CommandLineItem + ?Sized> CommandLineItem for &T {
    fn expand_to_command_line(&self) -> String {
        (**self).expand_to_command_line()
    }
}

impl<T: CommandLineItem + ?Sized> CommandLineItem for std::sync::Arc<T> {
    fn expand_to_command_line(&self) -> String {
        (**self).expand_to_command_line()
    }
}

macro_rules! to_string_command_line_item {
    ($($t:ty),*) => {
        $(impl CommandLineItem for $t {
            fn expand_to_command_line(&self) -> String {
                self.to_string()
            }
        })*
    };
}
to_string_command_line_item!(i32, i64, bool);

/// Thrown by command line classes during expansion.
///
/// This exception should be thrown deterministically, i.e. the command line should fail to expand
/// in exactly the same way every time at attempt is made. An example would be an illegal format
/// string, or a failure in Starlark evaluation of a compact command line.
// Java: com/google/devtools/build/lib/actions/CommandLineExpansionException.java:26 CommandLineExpansionException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLineExpansionException {
    pub message: String,
}

impl CommandLineExpansionException {
    pub fn new(user_visible_error_message: impl Into<String>) -> Self {
        CommandLineExpansionException {
            message: user_visible_error_message.into(),
        }
    }
}

impl fmt::Display for CommandLineExpansionException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CommandLineExpansionException {}

/// Java `InterruptedException`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterruptedException;

impl fmt::Display for InterruptedException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InterruptedException")
    }
}

impl std::error::Error for InterruptedException {}

/// The checked exceptions of `MapFn.expandToCommandLine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapFnError {
    CommandLineExpansion(CommandLineExpansionException),
    Interrupted(InterruptedException),
}

impl fmt::Display for MapFnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MapFnError::CommandLineExpansion(e) => e.fmt(f),
            MapFnError::Interrupted(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for MapFnError {}

impl From<CommandLineExpansionException> for MapFnError {
    fn from(e: CommandLineExpansionException) -> Self {
        MapFnError::CommandLineExpansion(e)
    }
}

impl From<InterruptedException> for MapFnError {
    fn from(e: InterruptedException) -> Self {
        MapFnError::Interrupted(e)
    }
}

/// A way of turning an arbitrary object into a command line string.
///
/// Implementations should be singletons (Java: `static final` instances; here `static` items or
/// leaked boxes, passed as `&'static`), or parametrized with [`ParametrizedMapFn`]. The nested
/// set fingerprint cache keys its memo tables on the map fn instance.
///
/// Plain closures and functions `Fn(&T, &mut dyn FnMut(&str))` are exceptionless map fns.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:23 CommandLineItem.MapFn
pub trait MapFn<T: ?Sized>: Any + Send + Sync {
    /// Expands the object into the command line. All string output is sent to `args`.
    // Java: com/google/devtools/build/lib/actions/CommandLineItem.java:28 MapFn.expandToCommandLine
    fn expand_to_command_line(
        &self,
        object: &T,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError>;

    /// Java: `mapFn instanceof CommandLineItem.ParametrizedMapFn`.
    fn as_parametrized_map_fn(&self) -> Option<&dyn ParametrizedMapFn> {
        None
    }

    /// Java: `mapFn.getClass().getName()`.
    // PORT: Rust type name, not the Java class name; only used in crash messages.
    fn class_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Use this map function when your map function can't throw an exception.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:33 CommandLineItem.ExceptionlessMapFn
pub trait ExceptionlessMapFn<T: ?Sized>: MapFn<T> {}

/// Use this map function when parametrizing over a limited set of values.
///
/// The user promises that the number of distinct instances constructed is closer to O(rule
/// class count) than O(rule count).
///
/// Without this, `NestedSetFingerprintCache` will refuse to cache your `MapFn` computations.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:48 CommandLineItem.ParametrizedMapFn
pub trait ParametrizedMapFn: Any + Send + Sync {
    /// Java `equals`. `other` is the other map fn (compare its concrete type first).
    fn equals(&self, other: &dyn Any) -> bool;

    /// Java `hashCode`.
    fn hash_code(&self) -> i32;

    /// This method controls the max number of arguments.
    ///
    /// Inform users if a custom implementation is used, to avoid misuse.
    fn max_instances_allowed(&self) -> i32;
}

/// Java: `CommandLineItem.MapFn.DEFAULT`, which expands each object with
/// `CommandLineItem.expandToCommandLine(object)`.
// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:24 MapFn.DEFAULT
pub struct DefaultMapFn;

pub static DEFAULT: DefaultMapFn = DefaultMapFn;

impl<T: CommandLineItem + ?Sized> MapFn<T> for DefaultMapFn {
    fn expand_to_command_line(
        &self,
        object: &T,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(&object.expand_to_command_line());
        Ok(())
    }
}

impl<T: CommandLineItem + ?Sized> ExceptionlessMapFn<T> for DefaultMapFn {}

impl<T: ?Sized, F> MapFn<T> for F
where
    F: Fn(&T, &mut dyn FnMut(&str)) + Send + Sync + 'static,
{
    fn expand_to_command_line(
        &self,
        object: &T,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        self(object, args);
        Ok(())
    }
}

impl<T: ?Sized, F> ExceptionlessMapFn<T> for F where
    F: Fn(&T, &mut dyn FnMut(&str)) + Send + Sync + 'static
{
}
