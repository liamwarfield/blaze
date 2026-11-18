//! Port of `com.google.devtools.build.lib.util.CommandDescriptionForm`.

/// Forms in which a command can be described by `CommandFailureUtils::describe_command`.
// Java: com/google/devtools/build/lib/util/CommandDescriptionForm.java:20 CommandDescriptionForm
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandDescriptionForm {
    /// A form that is usually suitable for identifying the command but not for re-executing it.
    /// The working directory and environment are not shown, and the arguments are truncated to a
    /// maximum of a few hundred bytes.
    Abbreviated,

    /// A form that is complete and suitable for a user to copy and paste into a shell. On Linux,
    /// the command is placed in a subshell so it has no side effects on the user's shell.
    Complete,
}
