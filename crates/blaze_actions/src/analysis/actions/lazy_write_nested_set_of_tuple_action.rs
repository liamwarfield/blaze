//! Port of `lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java`.

use std::any::Any;
use std::io::Write;
use std::sync::OnceLock;

use blaze_collect::command_line_item::CommandLineItem;
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;

use super::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, new_abstract_file_write_action,
};

/// A Starlark `Tuple` as this action reads it.
///
/// PORT: Java holds `net.starlark.java.eval.Tuple` values. The action only uses
/// `String.valueOf(element)` of each element (`StringBuilder.append(Object)`) and, for the key,
/// `Tuple.toString()` (= `Starlark.repr(tuple)`, through `CommandLineItem.expandToCommandLine`).
/// The caller computes both from the frozen Starlark value (`to_str()` of each element and
/// `to_repr()` of the tuple). Equality uses the repr, which distinguishes the same values Starlark
/// equality does for the string and int tuples Bazel stores here.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tuple {
    elements: Vec<String>,
    repr: String,
}

impl Tuple {
    /// `elements`: `String.valueOf` of each element; `repr`: `Starlark.repr(tuple)`.
    pub fn new(elements: Vec<String>, repr: String) -> Tuple {
        Tuple { elements, repr }
    }

    // Java: net/starlark/java/eval/Tuple.java Tuple.isEmpty
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    // Java: net/starlark/java/eval/Tuple.java Tuple.size
    pub fn size(&self) -> usize {
        self.elements.len()
    }

    /// `String.valueOf(tuple.get(i))`.
    // Java: net/starlark/java/eval/Tuple.java Tuple.get
    pub fn get(&self, i: usize) -> &str {
        &self.elements[i]
    }
}

// Java: com/google/devtools/build/lib/actions/CommandLineItem.java:75 CommandLineItem.expandToCommandLine
// (falls back on `Tuple.toString()`, which is `Starlark.repr(this)`).
impl CommandLineItem for Tuple {
    fn expand_to_command_line(&self) -> String {
        self.repr.clone()
    }
}

/// Lazily writes the content of a nested set of tuplesToWrite to an output file.
///
/// Writes delimiter separated Tuple elements to the output file.
// Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:38 LazyWriteNestedSetOfTupleAction
#[derive(Debug)]
pub struct LazyWriteNestedSetOfTupleAction {
    base: AbstractAction,
    tuples_to_write: NestedSet<Tuple>,
    file_contents: OnceLock<String>,
    delimiter: String,
}

impl LazyWriteNestedSetOfTupleAction {
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:44 LazyWriteNestedSetOfTupleAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        tuples_to_write: NestedSet<Tuple>,
        delimiter: impl Into<String>,
    ) -> LazyWriteNestedSetOfTupleAction {
        LazyWriteNestedSetOfTupleAction {
            base: new_abstract_file_write_action(
                owner,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                output,
            ),
            tuples_to_write,
            file_contents: OnceLock::new(),
            delimiter: delimiter.into(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:66 LazyWriteNestedSetOfTupleAction.getContents
    fn get_contents(&self, delimiter: &str) -> &str {
        self.file_contents.get_or_init(|| {
            let mut string_builder = String::new();
            for tuple in self.tuples_to_write.to_list().iter() {
                if tuple.is_empty() {
                    continue;
                }
                string_builder.push_str(tuple.get(0));
                for i in 1..tuple.size() {
                    string_builder.push_str(delimiter);
                    string_builder.push_str(tuple.get(i));
                }
                string_builder.push('\n');
            }
            string_builder
        })
    }

    /// The contents written to the output file.
    pub fn get_file_contents(&self) -> String {
        self.get_contents(&self.delimiter).to_string()
    }
}

impl Action for LazyWriteNestedSetOfTupleAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:96 AbstractFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        abstract_file_write_action::MNEMONIC.to_string()
    }

    /// Computes the Action key for this action by computing the fingerprint for the file
    /// contents.
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:58 LazyWriteNestedSetOfTupleAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        action_key_context.add_nested_set_to_fingerprint(fp, &self.tuples_to_write)?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }
}

impl AbstractFileWriteAction for LazyWriteNestedSetOfTupleAction {
    // Java: com/google/devtools/build/lib/analysis/actions/LazyWriteNestedSetOfTupleAction.java:52 LazyWriteNestedSetOfTupleAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(|out: &mut dyn Write| {
            // Java: `out.write(StringUnsafe.getInternalStringBytes(getContents(delimiter)))`.
            out.write_all(&blaze_util::string_encoding::latin1_encode(
                self.get_contents(&self.delimiter),
            ))
        })))
    }
}
