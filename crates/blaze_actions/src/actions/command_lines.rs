//! Port of `lib/actions/CommandLines.java`.

use std::io::{self, Write};
use std::sync::Arc;

use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;

use super::action::{CommandLineError, InputMetadataProvider};
use super::action_key_context::ActionKeyContext;
use super::artifact::Artifact;
use super::command_line::{self, CommandLine, CommandLineRef};
use super::command_line_limits::CommandLineLimits;
use super::param_file_info::ParamFileInfo;
use super::parameter_file::{self, ParameterFileType};
use super::path_mapper::{NOOP, OutputPathsMode, PathMapper};
use super::single_string_arg_formatter;

/// A (hopefully) conservative estimate of how much long each param file arg would be eg. the
/// length of '@path/to/param_file'.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:45 CommandLines.PARAM_FILE_ARG_LENGTH_ESTIMATE
const PARAM_FILE_ARG_LENGTH_ESTIMATE: i32 = 512;

/// `UUID.fromString("106c1389-88d7-4cc1-8f05-f8a61fd8f7b1")` as (least, most) significant bits.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:46 CommandLines.PARAM_FILE_UUID
const PARAM_FILE_UUID_LSB: i64 = 0x8f05_f8a6_1fd8_f7b1_u64 as i64;
const PARAM_FILE_UUID_MSB: i64 = 0x106c_1389_88d7_4cc1_u64 as i64;

/// A simple tuple of a [`CommandLine`] and a [`ParamFileInfo`].
// Java: com/google/devtools/build/lib/actions/CommandLines.java:50 CommandLines.CommandLineAndParamFileInfo
#[derive(Clone, Debug)]
pub struct CommandLineAndParamFileInfo {
    pub command_line: CommandLineRef,
    pub param_file_info: Option<ParamFileInfo>,
}

impl CommandLineAndParamFileInfo {
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:54 CommandLineAndParamFileInfo.<init>
    pub fn new(
        command_line: CommandLineRef,
        param_file_info: Option<ParamFileInfo>,
    ) -> CommandLineAndParamFileInfo {
        CommandLineAndParamFileInfo {
            command_line,
            param_file_info,
        }
    }
}

/// A single argument added with `Builder.addSingleArgument(Object)`, wrapped in a
/// `SingletonCommandLine` on unpacking.
///
/// PORT: Java accepts any `Object`; the kinds that reach it are strings, artifacts and path
/// fragments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SingleArgument {
    String(String),
    Artifact(Artifact),
    PathFragment(PathFragment),
}

/// Java `Object part`: a command line, or an arbitrary object to be wrapped in a
/// `SingletonCommandLine`.
#[derive(Clone, Debug)]
enum Part {
    CommandLine(CommandLineRef),
    Single(SingleArgument),
}

/// An element of `NPartCommandLines.commandLines`: a part, or the `ParamFileInfo` following a
/// command line.
#[derive(Clone, Debug)]
enum NPartElement {
    Part(Part),
    ParamFileInfo(ParamFileInfo),
}

/// The sealed subclasses of `CommandLines` (memory-optimized shapes).
#[derive(Clone, Debug)]
enum Repr {
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:394 CommandLines.OnePartCommandLines
    OnePart(Part),
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:408 CommandLines.TwoPartCommandLines
    TwoPart(Part, Part, Option<ParamFileInfo>),
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:427 CommandLines.ThreePartCommandLinesWithoutParamsFiles
    ThreePartWithoutParamsFiles(Part, Part, Part),
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:447 CommandLines.ThreePartCommandLines
    ThreePart(
        Part,
        Part,
        Option<ParamFileInfo>,
        Part,
        Option<ParamFileInfo>,
    ),
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:476 CommandLines.NPartCommandLines
    NPart(Arc<[NPartElement]>),
}

/// A class that keeps a list of command lines and optional associated parameter file info.
///
/// This class is used by `SpawnRunner` implementations to expand the command lines into a master
/// argument list + any param files needed to be written.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:41 CommandLines
#[derive(Clone, Debug)]
pub struct CommandLines(Repr);

impl CommandLines {
    /// Expands this object into a single primary command line and (0-N) param files. The spawn
    /// runner is expected to write these param files prior to execution of an action.
    ///
    /// * `input_metadata_provider`: the metadata provider to expand composite artifacts
    /// * `param_file_base_path`: Used to derive param file names. Often the first output of an
    ///   action
    /// * `path_mapper`: function to map configuration prefixes in output paths to more
    ///   cache-friendly identifiers
    /// * `limits`: The command line limits the host OS can support.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:74 CommandLines.expand
    pub fn expand(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        param_file_base_path: &PathFragment,
        path_mapper: &dyn PathMapper,
        limits: &CommandLineLimits,
    ) -> Result<ExpandedCommandLines, CommandLineError> {
        self.expand_with_estimate(
            input_metadata_provider,
            param_file_base_path,
            limits,
            path_mapper,
            PARAM_FILE_ARG_LENGTH_ESTIMATE,
        )
    }

    /// `expand` with an explicit param file argument length estimate (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:89 CommandLines.expand
    pub fn expand_with_estimate(
        &self,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        param_file_base_path: &PathFragment,
        limits: &CommandLineLimits,
        path_mapper: &dyn PathMapper,
        param_file_arg_length_estimate: i32,
    ) -> Result<ExpandedCommandLines, CommandLineError> {
        let command_lines = self.unpack();
        let mut arguments: Vec<String> = Vec::new();
        let mut param_files: Vec<ParamFileActionInput> = Vec::with_capacity(command_lines.len());
        // Java int arithmetic.
        let conservative_max_length: i32 = (limits.max_length as i32).wrapping_sub(
            (command_lines.len() as i32).wrapping_mul(param_file_arg_length_estimate),
        );
        let mut cmd_line_length: i32 = 0;
        // We name based on the output, starting at <output>-0.params and then incrementing
        let mut param_file_name_suffix: i32 = 0;
        for pair in &command_lines {
            let command_line = &pair.command_line;
            let param_file_info = pair.param_file_info.as_ref();
            let chunk = command_line.expand(input_metadata_provider, path_mapper)?;
            match param_file_info {
                None => {
                    arguments.extend(chunk.arguments(path_mapper));
                    cmd_line_length =
                        cmd_line_length.wrapping_add(chunk.total_arg_length(path_mapper));
                }
                Some(param_file_info) => {
                    let mut use_param_file = true;
                    if !param_file_info.always() {
                        let tentative_cmd_line_length =
                            cmd_line_length.wrapping_add(chunk.total_arg_length(path_mapper));
                        if tentative_cmd_line_length <= conservative_max_length {
                            arguments.extend(chunk.arguments(path_mapper));
                            cmd_line_length = tentative_cmd_line_length;
                            use_param_file = false;
                        }
                    }
                    if use_param_file {
                        let param_file_exec_path = parameter_file::derive_path_with_flavor(
                            param_file_base_path,
                            &param_file_name_suffix.to_string(),
                        );
                        param_file_name_suffix += 1;

                        let param_arg = single_string_arg_formatter::format(
                            param_file_info.get_flag_format_string(),
                            path_mapper.map(&param_file_exec_path).get_path_string(),
                        );
                        // Java `String.length()` of an internal string.
                        cmd_line_length =
                            cmd_line_length.wrapping_add(param_arg.chars().count() as i32 + 1);
                        arguments.push(param_arg);

                        if param_file_info.flags_only() {
                            // Move just the flags into the file, and keep the positional
                            // parameters on the command line.
                            param_files.push(ParamFileActionInput::new(
                                param_file_exec_path,
                                parameter_file::flags_only(chunk.arguments(path_mapper)),
                                param_file_info.get_file_type(),
                            ));
                            for positional_arg in
                                parameter_file::non_flags(chunk.arguments(path_mapper))
                            {
                                cmd_line_length = cmd_line_length
                                    .wrapping_add(positional_arg.chars().count() as i32 + 1);
                                arguments.push(positional_arg);
                            }
                        } else {
                            param_files.push(ParamFileActionInput::new(
                                param_file_exec_path,
                                chunk.arguments(path_mapper),
                                param_file_info.get_file_type(),
                            ));
                        }
                    }
                }
            }
        }
        Ok(ExpandedCommandLines {
            arguments,
            param_files,
        })
    }

    /// Returns all arguments, including ones inside of param files.
    ///
    /// Suitable for debugging and printing messages to users. This expands all command lines, so
    /// it is potentially expensive.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:161 CommandLines.allArguments
    pub fn all_arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.all_arguments_with(&NOOP)
    }

    /// Variation of [`CommandLines::all_arguments`] that supports output path stripping.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:167 CommandLines.allArguments
    pub fn all_arguments_with(
        &self,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let mut arguments = Vec::new();
        for pair in self.unpack() {
            arguments.extend(
                pair.command_line
                    .arguments_with(/* input_metadata_provider= */ None, path_mapper)?,
            );
        }
        Ok(arguments)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:176 CommandLines.addToFingerprint
    pub fn add_to_fingerprint(
        &self,
        action_key_context: &ActionKeyContext,
        input_metadata_provider: Option<&dyn InputMetadataProvider>,
        effective_output_paths_mode: OutputPathsMode,
        fingerprint: &mut Fingerprint,
    ) -> Result<(), CommandLineError> {
        let command_lines = self.unpack();
        for pair in &command_lines {
            pair.command_line.add_to_fingerprint(
                action_key_context,
                input_metadata_provider,
                effective_output_paths_mode,
                fingerprint,
            )?;
            if let Some(param_file_info) = &pair.param_file_info {
                add_param_file_info_to_fingerprint(param_file_info, fingerprint);
            }
        }
        Ok(())
    }

    /// Unpacks the optimized storage format into a list of [`CommandLineAndParamFileInfo`].
    ///
    /// The returned list and its elements are not part of the optimized storage representation.
    /// Retaining them in an action would defeat the memory optimizations made by `CommandLines`.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:267 CommandLines.unpack
    pub fn unpack(&self) -> Vec<CommandLineAndParamFileInfo> {
        match &self.0 {
            // Java: com/google/devtools/build/lib/actions/CommandLines.java:401 OnePartCommandLines.unpack
            Repr::OnePart(part1) => vec![CommandLineAndParamFileInfo::new(
                to_executable_command_line(part1),
                None,
            )],
            // Java: com/google/devtools/build/lib/actions/CommandLines.java:419 TwoPartCommandLines.unpack
            Repr::TwoPart(part1, part2, part2_param_file_info) => vec![
                CommandLineAndParamFileInfo::new(to_executable_command_line(part1), None),
                CommandLineAndParamFileInfo::new(
                    to_non_executable_command_line(part2),
                    part2_param_file_info.clone(),
                ),
            ],
            // Java: com/google/devtools/build/lib/actions/CommandLines.java:438 ThreePartCommandLinesWithoutParamsFiles.unpack
            Repr::ThreePartWithoutParamsFiles(part1, part2, part3) => vec![
                CommandLineAndParamFileInfo::new(to_executable_command_line(part1), None),
                CommandLineAndParamFileInfo::new(to_non_executable_command_line(part2), None),
                CommandLineAndParamFileInfo::new(to_non_executable_command_line(part3), None),
            ],
            // Java: com/google/devtools/build/lib/actions/CommandLines.java:467 ThreePartCommandLines.unpack
            Repr::ThreePart(part1, part2, part2_param_file_info, part3, part3_param_file_info) => {
                vec![
                    CommandLineAndParamFileInfo::new(to_executable_command_line(part1), None),
                    CommandLineAndParamFileInfo::new(
                        to_non_executable_command_line(part2),
                        part2_param_file_info.clone(),
                    ),
                    CommandLineAndParamFileInfo::new(
                        to_non_executable_command_line(part3),
                        part3_param_file_info.clone(),
                    ),
                ]
            }
            // Java: com/google/devtools/build/lib/actions/CommandLines.java:493 NPartCommandLines.unpack
            Repr::NPart(command_lines) => {
                let mut result = Vec::new();
                let mut i = 0usize;
                while i < command_lines.len() {
                    let obj = &command_lines[i];
                    let command_line: CommandLineRef;
                    let mut param_file_info: Option<ParamFileInfo> = None;
                    match obj {
                        NPartElement::Part(Part::CommandLine(c)) => {
                            command_line = Arc::clone(c);
                            if i + 1 < command_lines.len() {
                                if let NPartElement::ParamFileInfo(p) = &command_lines[i + 1] {
                                    i += 1;
                                    param_file_info = Some(p.clone());
                                }
                            }
                        }
                        NPartElement::Part(Part::Single(arg)) => {
                            command_line = Arc::new(SingletonCommandLine {
                                arg: arg.clone(),
                                has_executable_path: i == 0,
                            });
                        }
                        // Java: a ParamFileInfo not preceded by a CommandLine is wrapped in a
                        // SingletonCommandLine; the builder never produces that layout.
                        NPartElement::ParamFileInfo(p) => {
                            panic!("IllegalStateException: unexpected ParamFileInfo {:?}", p)
                        }
                    }
                    result.push(CommandLineAndParamFileInfo::new(
                        command_line,
                        param_file_info,
                    ));
                    i += 1;
                }
                result
            }
        }
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:276 CommandLines.builder
    pub fn builder() -> CommandLinesBuilder {
        CommandLinesBuilder {
            part1: None,
            part2: None,
            part2_param_file_info: None,
            part3: None,
            part3_param_file_info: None,
            parts: 0,
            command_lines: Vec::new(),
        }
    }

    /// Returns an instance with a single command line.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:281 CommandLines.of
    pub fn of(command_line: CommandLineRef) -> CommandLines {
        CommandLines(Repr::OnePart(Part::CommandLine(command_line)))
    }

    /// Returns an instance with a single trivial command line.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:286 CommandLines.of
    pub fn of_args(args: Vec<String>) -> CommandLines {
        CommandLines(Repr::OnePart(Part::CommandLine(command_line::of(args))))
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:290 CommandLines.concat
    pub fn concat(command_line: CommandLineRef, command_lines: &CommandLines) -> CommandLines {
        let mut builder = CommandLines::builder();
        builder.add_command_line(command_line);
        for pair in command_lines.unpack() {
            builder.add_command_line_and_param_file_info(pair);
        }
        builder.build()
    }
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:269 CommandLines.addParamFileInfoToFingerprint
fn add_param_file_info_to_fingerprint(
    param_file_info: &ParamFileInfo,
    fingerprint: &mut Fingerprint,
) {
    fingerprint.add_uuid(PARAM_FILE_UUID_LSB, PARAM_FILE_UUID_MSB);
    // Invariant: the flag format string (it may come from Starlark) is an internal string (see
    // `fingerprint_util`).
    fingerprint.add_string(param_file_info.get_flag_format_string());
    fingerprint.add_string(param_file_info.get_file_type().name());
}

/// Builder for [`CommandLines`].
///
/// Attempts to build the most memory-efficient `CommandLines` instance possible. Most command
/// lines are composed of 1-3 parts. Additionally, the first part is typically just an executable
/// or shell command and does not have an associated params file. If both of these criteria are
/// met, memory is saved by using one of the array-free subclasses. Otherwise, uses
/// `NPartCommandLines` which handles any arbitrary case.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:308 CommandLines.Builder
#[derive(Clone, Debug)]
pub struct CommandLinesBuilder {
    /// Set to `None` when we need to use NPartCommandLines.
    part1: Option<Part>,
    part2: Option<Part>,
    part2_param_file_info: Option<ParamFileInfo>,
    part3: Option<Part>,
    part3_param_file_info: Option<ParamFileInfo>,
    parts: i32,
    command_lines: Vec<NPartElement>,
}

impl CommandLinesBuilder {
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:318 CommandLines.Builder.addSingleArgument
    pub fn add_single_argument(&mut self, argument: impl Into<String>) -> &mut Self {
        self.add_internal(Part::Single(SingleArgument::String(argument.into())), None)
    }

    /// `addSingleArgument(artifact)`.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:318 CommandLines.Builder.addSingleArgument
    pub fn add_single_artifact(&mut self, argument: Artifact) -> &mut Self {
        self.add_internal(Part::Single(SingleArgument::Artifact(argument)), None)
    }

    /// `addSingleArgument(pathFragment)`.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:318 CommandLines.Builder.addSingleArgument
    pub fn add_single_path_fragment(&mut self, argument: PathFragment) -> &mut Self {
        self.add_internal(Part::Single(SingleArgument::PathFragment(argument)), None)
    }

    /// `addSingleArgument(Object)` with an already classified argument.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:318 CommandLines.Builder.addSingleArgument
    pub fn add_single(&mut self, argument: SingleArgument) -> &mut Self {
        // Java: checkArgument(!(argument instanceof ParamFileInfo) && !(argument instanceof
        // CommandLineAndParamFileInfo)); the type of `argument` guarantees it.
        self.add_internal(Part::Single(argument), None)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:327 CommandLines.Builder.addCommandLine
    pub fn add_command_line(&mut self, command_line: CommandLineRef) -> &mut Self {
        self.add_internal(Part::CommandLine(command_line), None)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:332 CommandLines.Builder.addCommandLine
    pub fn add_command_line_with_param_file(
        &mut self,
        command_line: CommandLineRef,
        param_file_info: Option<ParamFileInfo>,
    ) -> &mut Self {
        self.add_internal(Part::CommandLine(command_line), param_file_info)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:337 CommandLines.Builder.addCommandLine
    pub fn add_command_line_and_param_file_info(
        &mut self,
        pair: CommandLineAndParamFileInfo,
    ) -> &mut Self {
        self.add_internal(Part::CommandLine(pair.command_line), pair.param_file_info)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:341 CommandLines.Builder.addInternal
    fn add_internal(&mut self, part: Part, param_file_info: Option<ParamFileInfo>) -> &mut Self {
        self.parts += 1;
        if self.parts == 1 {
            if param_file_info.is_none() {
                self.part1 = Some(part.clone());
            }
        } else if self.parts == 2 {
            self.part2 = Some(part.clone());
            self.part2_param_file_info = param_file_info.clone();
        } else if self.parts == 3 {
            self.part3 = Some(part.clone());
            self.part3_param_file_info = param_file_info.clone();
        } else if self.parts == 4 {
            self.part1 = None; // Destined to build an NPartCommandLines.
        }
        self.command_lines.push(NPartElement::Part(part));
        if let Some(p) = param_file_info {
            self.command_lines.push(NPartElement::ParamFileInfo(p));
        }
        self
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:363 CommandLines.Builder.build
    pub fn build(&self) -> CommandLines {
        let part1 = match &self.part1 {
            None => {
                return CommandLines(Repr::NPart(self.command_lines.clone().into()));
            }
            Some(p) => p.clone(),
        };
        if self.parts == 1 {
            return CommandLines(Repr::OnePart(part1));
        }
        let part2 = self.part2.clone().expect("part2");
        if self.parts == 2 {
            return CommandLines(Repr::TwoPart(
                part1,
                part2,
                self.part2_param_file_info.clone(),
            ));
        }
        let part3 = self.part3.clone().expect("part3");
        if self.part2_param_file_info.is_none() && self.part3_param_file_info.is_none() {
            return CommandLines(Repr::ThreePartWithoutParamsFiles(part1, part2, part3));
        }
        CommandLines(Repr::ThreePart(
            part1,
            part2,
            self.part2_param_file_info.clone(),
            part3,
            self.part3_param_file_info.clone(),
        ))
    }
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:380 CommandLines.toExecutableCommandLine
fn to_executable_command_line(obj: &Part) -> CommandLineRef {
    to_command_line(obj, /* has_executable_path= */ true)
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:384 CommandLines.toNonExecutableCommandLine
fn to_non_executable_command_line(obj: &Part) -> CommandLineRef {
    to_command_line(obj, /* has_executable_path= */ false)
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:388 CommandLines.toCommandLine
fn to_command_line(obj: &Part, has_executable_path: bool) -> CommandLineRef {
    match obj {
        Part::CommandLine(command_line) => Arc::clone(command_line),
        Part::Single(arg) => Arc::new(SingletonCommandLine {
            arg: arg.clone(),
            has_executable_path,
        }),
    }
}

/// Expanded command lines.
///
/// The spawn runner implementation is expected to ensure the param files are available once the
/// spawn is executed.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:200 CommandLines.ExpandedCommandLines
#[derive(Clone, Debug)]
pub struct ExpandedCommandLines {
    arguments: Vec<String>,
    param_files: Vec<ParamFileActionInput>,
}

impl ExpandedCommandLines {
    /// Returns the primary command line of the command.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:210 ExpandedCommandLines.arguments
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    /// Returns the param file action inputs needed to execute the command.
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:215 ExpandedCommandLines.getParamFiles
    pub fn get_param_files(&self) -> &[ParamFileActionInput] {
        &self.param_files
    }
}

/// An in-memory param file virtual action input.
// Java: com/google/devtools/build/lib/actions/CommandLines.java:221 CommandLines.ParamFileActionInput
// PORT: `VirtualActionInput` (the execution-time base class) is not ported; its
// `atomicallyWriteTo` belongs to the execution layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamFileActionInput {
    param_file_exec_path: PathFragment,
    arguments: Vec<String>,
    type_: ParameterFileType,
}

impl ParamFileActionInput {
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:226 ParamFileActionInput.<init>
    pub fn new(
        param_file_exec_path: PathFragment,
        arguments: Vec<String>,
        type_: ParameterFileType,
    ) -> ParamFileActionInput {
        ParamFileActionInput {
            param_file_exec_path,
            arguments,
            type_,
        }
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:234 ParamFileActionInput.writeTo
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        parameter_file::write_parameter_file(out, &self.arguments, self.type_)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:246 ParamFileActionInput.getExecPathString
    pub fn get_exec_path_string(&self) -> &str {
        self.param_file_exec_path.get_path_string()
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:251 ParamFileActionInput.getExecPath
    pub fn get_exec_path(&self) -> &PathFragment {
        &self.param_file_exec_path
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:255 ParamFileActionInput.getArguments
    pub fn get_arguments(&self) -> &[String] {
        &self.arguments
    }

    /// The parameter file type (Java's private `type` field).
    pub fn get_type(&self) -> ParameterFileType {
        self.type_
    }
}

// Java: com/google/devtools/build/lib/actions/CommandLines.java:515 CommandLines.SingletonCommandLine
#[derive(Debug)]
struct SingletonCommandLine {
    arg: SingleArgument,
    has_executable_path: bool,
}

impl CommandLine for SingletonCommandLine {
    // Java: com/google/devtools/build/lib/actions/CommandLines.java:525 SingletonCommandLine.arguments
    fn arguments(&self) -> Result<Vec<String>, CommandLineError> {
        self.arguments_with(None, &NOOP)
    }

    // Java: com/google/devtools/build/lib/actions/CommandLines.java:530 SingletonCommandLine.arguments
    fn arguments_with(
        &self,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        path_mapper: &dyn PathMapper,
    ) -> Result<Vec<String>, CommandLineError> {
        let s = match &self.arg {
            // Java: `case PathStrippable ps -> ps.expand(pathMapper::map)`; only derived
            // artifacts and path fragments are PathStrippable.
            // Java: com/google/devtools/build/lib/actions/Artifact.java:322 Artifact.DerivedArtifact.expand
            SingleArgument::Artifact(a) if a.is_derived_artifact_instance() => path_mapper
                .map(a.get_exec_path())
                .get_path_string()
                .to_string(),
            SingleArgument::PathFragment(p) => path_mapper.map(p).get_path_string().to_string(),
            // StarlarkAction stores the executable path as a string to save memory, but it
            // should still be mapped just like a PathFragment. In this case, the string always
            // represents a normalized path, so if it isn't (e.g. because it is an absolute path,
            // possibly for another OS), don't normalize or map it.
            SingleArgument::String(s) if self.has_executable_path => {
                let path_fragment = PathFragment::create(s);
                if path_fragment.get_path_string() != s {
                    s.clone()
                } else {
                    path_mapper
                        .map(&path_fragment)
                        .get_path_string()
                        .to_string()
                }
            }
            // Java: `default -> CommandLineItem.expandToCommandLine(arg)`.
            SingleArgument::String(s) => s.clone(),
            SingleArgument::Artifact(a) => a.expand_to_command_line(),
        };
        Ok(vec![s])
    }
}
