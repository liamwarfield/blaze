//! Port of `CompiledModuleFile` (with its `SyntaxChecker`, a `DotBazelFileSyntaxChecker`
//! subclass). OWNER: agent A.
//!
//! PORT: Java's `Program.compileFile` resolves every identifier and rejects undefined names
//! statically ("name 'x' is not defined"). starlark-rust reports them only when the module is
//! evaluated, so [`resolve_names`] ports that part of `Resolver` for MODULE.bazel files (top-level
//! globals, comprehension scopes, predeclared and universal names), together with
//! `blaze_starlark::syntax::resolver` (rebinding, load placement).

use std::fmt;

use blaze_bzlmod_model::{ExternalDepsCode, ExternalDepsException, ModuleFile, ModuleKey};
use blaze_events::{Event, ExtendedEventHandler, Location};
use blaze_options::spelling::SpellChecker;
use blaze_starlark::eval::DEFAULT_MAX_CALLSTACK;
use blaze_starlark::semantics::Utf8EnforcementMode;
use blaze_starlark::{EvalError, FileKind, StarlarkSemantics, SyntaxError, ThreadContext};
use indexmap::IndexSet;
use starlark::codemap::Span;
use starlark::environment::{Globals, Module};
use starlark::eval::Evaluator;
use starlark::syntax::AstModule;
use starlark::syntax::ast::{
    ArgumentP, AssignTargetP, AstAssignTarget, AstExpr, AstLiteral, AstStmt, ClauseP, ExprP,
    ForClauseP, StmtP,
};

use super::java_hash::java_hash_order;
use super::module_file_globals::{java_eval_error, module_file_globals};
use super::module_thread_context::ModuleThreadContext;
use super::to_events_location;

/// A top-level `include("//label", dev_dependency = ...)` statement.
// Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:56 CompiledModuleFile.IncludeStatement
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeStatement {
    pub include_label: String,
    pub dev_dependency: bool,
    pub location: Location,
}

/// A parsed and syntax-checked MODULE.bazel file, ready to be executed. `Send + Sync + Clone`.
///
/// PORT: Java keeps a compiled `Program` and its `predeclaredEnv` module. Here the AST and the
/// MODULE.bazel `Globals` are kept, and every run evaluates the AST in a fresh starlark-rust
/// module (an included file can run several times).
// Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:49 CompiledModuleFile
#[derive(Clone)]
pub struct CompiledModuleFile {
    pub module_file: ModuleFile,
    pub include_statements: Vec<IncludeStatement>,
    program: AstModule,
    predeclared_env: Globals,
}

impl fmt::Debug for CompiledModuleFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledModuleFile")
            .field("module_file", &self.module_file)
            .field("include_statements", &self.include_statements)
            .finish_non_exhaustive()
    }
}

// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:27 StarlarkUtil.INVALID_UTF_8_MESSAGE
const INVALID_UTF_8_MESSAGE: &str = "not a valid UTF-8 encoded file; this can lead to inconsistent behavior and will be disallowed in a future version of Bazel";

/// Java `StarlarkUtil.InvalidUtf8Exception`.
#[derive(Debug)]
struct InvalidUtf8Exception;

/// Checks the file's encoding per `--incompatible_enforce_starlark_utf8` and returns its text.
// PORT: Java decodes the bytes as Latin-1 (Bazel's internal string form); valid UTF-8 is
// the same text in Rust (StringEncoding identity). Non-UTF-8 content is not ported.
// Java: com/google/devtools/build/lib/skyframe/StarlarkUtil.java:43 StarlarkUtil.createParserInput
fn create_parser_input(
    bytes: &[u8],
    file: &str,
    utf8_enforcement_mode: Utf8EnforcementMode,
    reporter: &dyn ExtendedEventHandler,
) -> Result<String, InvalidUtf8Exception> {
    let well_formed = std::str::from_utf8(bytes).is_ok();
    match utf8_enforcement_mode {
        Utf8EnforcementMode::Off => {}
        Utf8EnforcementMode::Warning => {
            if !well_formed {
                reporter.handle(&Event::warn_at(
                    Some(Location::from_file(file)),
                    INVALID_UTF_8_MESSAGE,
                ));
            }
        }
        Utf8EnforcementMode::Error => {
            if !well_formed {
                reporter.handle(&Event::error_at(
                    Some(Location::from_file(file)),
                    format!(
                        "{}. For a temporary workaround, see the --{} flag.",
                        INVALID_UTF_8_MESSAGE, "incompatible_enforce_starlark_utf8"
                    ),
                ));
                return Err(InvalidUtf8Exception);
            }
        }
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => Ok(s),
        Err(_) => blaze_util::unported!("ParserInput.fromLatin1", "non-UTF-8 file {file}"),
    }
}

fn replay_syntax_errors(event_handler: &dyn ExtendedEventHandler, errors: &[SyntaxError]) {
    // Java: Event.replayEventsOn(eventHandler, errors)
    for e in errors {
        event_handler.handle(&Event::error_at(
            Some(to_events_location(&e.location)),
            e.message.clone(),
        ));
    }
}

impl CompiledModuleFile {
    pub const INCLUDE_IDENTIFIER: &'static str = "include";

    /// Parses and compiles a given module file, checking it for syntax errors. Errors are
    /// replayed on `event_handler`; the returned exception is BAD_MODULE with Java's message.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:59 CompiledModuleFile.parseAndCompile
    pub fn parse_and_compile(
        module_file: ModuleFile,
        module_key: &ModuleKey,
        starlark_semantics: &StarlarkSemantics,
        event_handler: &dyn ExtendedEventHandler,
    ) -> Result<CompiledModuleFile, ExternalDepsException> {
        let parser_input = match create_parser_input(
            &module_file.content,
            &module_file.location,
            starlark_semantics.incompatible_enforce_starlark_utf8,
            event_handler,
        ) {
            Ok(input) => input,
            // createParserInput() reports its own error message
            Err(InvalidUtf8Exception) => {
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!("error reading MODULE.bazel file for {module_key}"),
                ));
            }
        };
        let starlark_file = match parse(&module_file.location, parser_input) {
            Ok(ast) => ast,
            Err(errors) => {
                replay_syntax_errors(event_handler, &errors);
                return Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!("error parsing MODULE.bazel file for {module_key}"),
                ));
            }
        };
        let compiled = check_module_file_syntax(&starlark_file).and_then(|include_statements| {
            let predeclared_env = module_file_globals(starlark_semantics);
            compile_file(&starlark_file, &predeclared_env)?;
            Ok((include_statements, predeclared_env))
        });
        match compiled {
            Ok((include_statements, predeclared_env)) => Ok(CompiledModuleFile {
                module_file,
                include_statements,
                program: starlark_file,
                predeclared_env,
            }),
            Err(errors) => {
                replay_syntax_errors(event_handler, &errors);
                Err(ExternalDepsException::with_message(
                    ExternalDepsCode::BadModule,
                    format!("syntax error in MODULE.bazel file for {module_key}"),
                ))
            }
        }
    }

    /// Executes the file on `thread` (whose payload is the [`ModuleThreadContext`]), in a fresh
    /// module with this file's predeclared environment. Errors carry the full Bazel call stack,
    /// including the frames of the files including this one.
    // Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:194 CompiledModuleFile.runOnThread
    pub(crate) fn run_on_thread(&self, thread: &ThreadContext) -> Result<(), EvalError> {
        let prefix = thread
            .payload::<ModuleThreadContext>()
            .map(|c| c.call_stack_prefix.clone())
            .unwrap_or_default();
        let program = self.program.clone();
        let globals = &self.predeclared_env;
        Module::with_temp_heap(|module| {
            let mut eval = Evaluator::new(&module);
            eval.extra = Some(thread);
            eval.set_forbid_recursion(!thread.semantics.allow_recursion);
            eval.set_max_callstack_size(DEFAULT_MAX_CALLSTACK)
                .expect("fresh evaluator");
            eval.eval_module(program, globals)
                .map(|_| ())
                .map_err(|e| java_eval_error(&e, &prefix))
        })
    }
}

/// Parses `source` with MODULE.bazel's dialect (Java `StarlarkFile.parse`); a parse error is a
/// single syntax error (starlark-rust stops at the first one).
fn parse(file_name: &str, source: String) -> Result<AstModule, Vec<SyntaxError>> {
    AstModule::parse(file_name, source, &FileKind::ModuleFile.dialect()).map_err(|e| {
        let location = e
            .span()
            .map(blaze_starlark::Location::of_span_start)
            .unwrap_or_else(|| blaze_starlark::Location {
                file: file_name.to_owned(),
                line: 0,
                column: 0,
            });
        vec![SyntaxError {
            location,
            message: blaze_starlark::error::bare_message(&e),
        }]
    })
}

/// `Program.compileFile`'s static checks: `blaze_starlark`'s resolver rules for MODULE.bazel's
/// `FileOptions`, then undefined names.
// Java: net/starlark/java/syntax/Program.java Program.compileFile
fn compile_file(ast: &AstModule, predeclared_env: &Globals) -> Result<(), Vec<SyntaxError>> {
    let mut errors =
        blaze_starlark::syntax::resolver::check(ast, FileKind::ModuleFile.file_options());
    errors.extend(resolve_names(ast, predeclared_env));
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Checks the given file for module file syntax, and returns the list of `include` statements it
/// contains. This is a somewhat crude sweep over the AST; we loudly complain about any usage of
/// `include` that is not in a top-level function call statement with one single string literal
/// positional argument and an optional boolean literal `dev_dependency` keyword argument,
/// *except* that we don't do this check once `include` is assigned to, due to backwards
/// compatibility concerns.
///
/// The error type is Java's `SyntaxError.Exception` (its message is
/// `blaze_starlark::error::syntax_errors_message`).
// Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:187 CompiledModuleFile.checkModuleFileSyntax
pub fn check_module_file_syntax(
    starlark_file: &AstModule,
) -> Result<Vec<IncludeStatement>, Vec<SyntaxError>> {
    let mut checker = SyntaxChecker {
        ast: starlark_file,
        errors: Vec::new(),
        include_statements: Vec::new(),
        include_was_assigned: false,
    };
    // Java: DotBazelFileSyntaxChecker.check
    checker.visit_stmt(starlark_file.statement());
    if checker.errors.is_empty() {
        Ok(checker.include_statements)
    } else {
        Err(checker.errors)
    }
}

/// `DotBazelFileSyntaxChecker("MODULE.bazel files", canLoadBzl=false,
/// allowLiteralStarStarArgs=true)` with `CompiledModuleFile.SyntaxChecker`'s overrides, in one
/// traversal in Java `NodeVisitor` order (with `skipNonSymbolIdentifiers`: keyword names and
/// dot fields are not visited, which starlark-rust's AST does not represent as identifiers).
// Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:99 CompiledModuleFile.SyntaxChecker
struct SyntaxChecker<'a> {
    ast: &'a AstModule,
    errors: Vec<SyntaxError>,
    include_statements: Vec<IncludeStatement>,
    // Once `include` the identifier is assigned to, we no longer care about its appearance
    // anywhere. This allows `include` to be used as a module extension proxy (and technically
    // any other variable binding).
    include_was_assigned: bool,
}

// Java: CompiledModuleFile.java:109 SyntaxChecker.<init>
const WHERE: &str = "MODULE.bazel files";
const CAN_LOAD_BZL: bool = false;
const ALLOW_LITERAL_STAR_STAR_ARGS: bool = true;

impl SyntaxChecker<'_> {
    fn location(&self, span: Span) -> blaze_starlark::Location {
        blaze_starlark::Location::of_span_start(&self.ast.file_span(span))
    }

    // Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java:81 DotBazelFileSyntaxChecker.error
    fn error(&mut self, span: Span, message: String) {
        let location = self.location(span);
        self.errors.push(SyntaxError { location, message });
    }

    fn functions_message() -> String {
        format!(
            "functions may not be defined in {}{}",
            WHERE,
            if CAN_LOAD_BZL {
                ". You may move the function to a .bzl file and load it."
            } else {
                "."
            }
        )
    }

    fn visit_stmt(&mut self, s: &AstStmt) {
        match &s.node {
            StmtP::Statements(xs) => {
                for x in xs {
                    self.visit_stmt(x);
                }
            }
            StmtP::Expression(e) => self.visit_expression_statement(s, e),
            StmtP::Assign(a) => self.visit_assignment(&a.lhs, &a.rhs),
            StmtP::AssignModify(lhs, _, rhs) => self.visit_assignment(lhs, rhs),
            // Java: DotBazelFileSyntaxChecker.java:110 visit(LoadStatement)
            StmtP::Load(_) => {
                if !CAN_LOAD_BZL {
                    self.error(s.span, format!("`load` statements may not be used in {WHERE}"));
                }
            }
            // We prune the traversal if we encounter disallowed keywords, as we have already
            // reported the root error and there's no point reporting more.
            // Java: DotBazelFileSyntaxChecker.java:120 visit(DefStatement)
            StmtP::Def(_) => self.error(s.span, Self::functions_message()),
            // Java: DotBazelFileSyntaxChecker.java:138 visit(ForStatement)
            StmtP::For(_) => self.error(
                s.span,
                format!(
                    "`for` statements are not allowed in {}. You may inline the loop{} or as a last resort use a list comprehension.",
                    WHERE,
                    if CAN_LOAD_BZL {
                        ", move it to a function definition (in a .bzl file),"
                    } else {
                        ""
                    }
                ),
            ),
            // Java: DotBazelFileSyntaxChecker.java:149 visit(IfStatement)
            StmtP::If(..) | StmtP::IfElse(..) => self.error(
                s.span,
                format!(
                    "`if` statements are not allowed in {}. You may{} use an `if` expression for simple cases.",
                    WHERE,
                    if CAN_LOAD_BZL {
                        " move conditional logic to a function definition (in a .bzl file), or"
                    } else {
                        ""
                    }
                ),
            ),
            // Java: NodeVisitor.visit(ReturnStatement)
            StmtP::Return(Some(e)) => self.visit_expr(e),
            StmtP::Return(None) | StmtP::Break | StmtP::Continue | StmtP::Pass => {}
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:115 SyntaxChecker.visit(ExpressionStatement)
    fn visit_expression_statement(&mut self, node: &AstStmt, expression: &AstExpr) {
        // We can assume this statement isn't nested in any block, since we don't allow
        // `if`/`def`/`for` in MODULE.bazel.
        if !self.include_was_assigned
            && let ExprP::Call(function, call_args) = &expression.node
            && let ExprP::Identifier(id) = &function.node
            && id.node.ident == CompiledModuleFile::INCLUDE_IDENTIFIER
        {
            // Found a top-level call to `include`!
            let args = &call_args.args;
            // `dev_dependency = True` / `dev_dependency = False`: the literal's name.
            let dev_dependency_literal = |arg: &starlark::syntax::ast::AstArgument| match &arg.node
            {
                ArgumentP::Named(keyword_name, keyword_value)
                    if keyword_name.node == "dev_dependency" =>
                {
                    match &keyword_value.node {
                        ExprP::Identifier(id)
                            if id.node.ident == "True" || id.node.ident == "False" =>
                        {
                            Some(id.node.ident.clone())
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(first) = args.first()
                && let ArgumentP::Positional(pos) = &first.node
                && let ExprP::Literal(AstLiteral::String(str_)) = &pos.node
                && (args.len() == 1
                    || (args.len() == 2 && dev_dependency_literal(&args[1]).is_some()))
            {
                let dev_dependency =
                    args.len() == 2 && dev_dependency_literal(&args[1]).as_deref() == Some("True");
                let location = to_events_location(&self.location(expression.span));
                self.include_statements.push(IncludeStatement {
                    include_label: str_.node.clone(),
                    dev_dependency,
                    location,
                });
                // Nothing else to check, we can stop visiting sub-nodes now.
                return;
            }
            self.error(
                node.span,
                "the `include` directive MUST be called with one positional string literal argument and, optionally, `dev_dependency` as a boolean literal".to_owned(),
            );
            return;
        }
        // Java: NodeVisitor.visit(ExpressionStatement)
        self.visit_expr(expression);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:151 SyntaxChecker.visit(AssignmentStatement)
    fn visit_assignment(&mut self, lhs: &AstAssignTarget, rhs: &AstExpr) {
        self.visit_expr(rhs);
        if !self.include_was_assigned
            && let AssignTargetP::Identifier(id) = &lhs.node
            && id.node.ident == CompiledModuleFile::INCLUDE_IDENTIFIER
        {
            self.include_was_assigned = true;
            // Technically someone could do something like
            //   (include, myvar) = (print, 3)
            // and work around our check, but at that point IDGAF.
        } else {
            self.visit_target(lhs);
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/CompiledModuleFile.java:166 SyntaxChecker.visit(Identifier)
    fn visit_identifier(&mut self, name: &str, span: Span) {
        if !self.include_was_assigned && name == CompiledModuleFile::INCLUDE_IDENTIFIER {
            // If we somehow reach the `include` identifier but NOT as the other allowed cases above,
            // cry foul.
            self.error(
                span,
                "the `include` directive MUST be called directly at the top-level".to_owned(),
            );
        }
    }

    /// An assignment target (Java visits the LHS expression).
    fn visit_target(&mut self, t: &AstAssignTarget) {
        match &t.node {
            AssignTargetP::Tuple(xs) => {
                for x in xs {
                    self.visit_target(x);
                }
            }
            AssignTargetP::Index(b) => {
                self.visit_expr(&b.0);
                self.visit_expr(&b.1);
            }
            // The field is not a symbol identifier.
            AssignTargetP::Dot(object, _) => self.visit_expr(object),
            AssignTargetP::Identifier(id) => self.visit_identifier(&id.node.ident, id.span),
        }
    }

    fn visit_for_clause(&mut self, clause: &ForClauseP<starlark::syntax::ast::AstNoPayload>) {
        // Java: NodeVisitor.visit(Comprehension.For)
        self.visit_target(&clause.var);
        self.visit_expr(&clause.over);
    }

    fn visit_clauses(
        &mut self,
        first: &ForClauseP<starlark::syntax::ast::AstNoPayload>,
        rest: &[ClauseP<starlark::syntax::ast::AstNoPayload>],
    ) {
        self.visit_for_clause(first);
        for clause in rest {
            match clause {
                ClauseP::For(f) => self.visit_for_clause(f),
                ClauseP::If(cond) => self.visit_expr(cond),
            }
        }
    }

    fn visit_expr(&mut self, e: &AstExpr) {
        match &e.node {
            ExprP::Identifier(id) => self.visit_identifier(&id.node.ident, e.span),
            // Java: DotBazelFileSyntaxChecker.java:129 visit(LambdaExpression); traversal pruned.
            ExprP::Lambda(_) => self.error(e.span, Self::functions_message()),
            // Java: DotBazelFileSyntaxChecker.java:162 visit(CallExpression)
            ExprP::Call(function, args) => {
                self.reject_star_args(&args.args);
                // Continue traversal so as not to miss nested calls
                // like cc_binary(..., f(**kwargs), ...).
                self.visit_expr(function);
                for arg in &args.args {
                    // Java: NodeVisitor.visit(Argument), keyword name skipped.
                    self.visit_expr(arg.node.expr());
                }
            }
            // The field is not a symbol identifier.
            ExprP::Dot(object, _) => self.visit_expr(object),
            // Java: NodeVisitor.visit(ConditionalExpression): then, condition, else.
            ExprP::If(b) => {
                let (cond, then_case, else_case) = &**b;
                self.visit_expr(then_case);
                self.visit_expr(cond);
                self.visit_expr(else_case);
            }
            // Java: NodeVisitor.visit(Comprehension): body, then clauses.
            ExprP::ListComprehension(body, first, rest) => {
                self.visit_expr(body);
                self.visit_clauses(first, rest);
            }
            ExprP::DictComprehension(body, first, rest) => {
                self.visit_expr(&body.0);
                self.visit_expr(&body.1);
                self.visit_clauses(first, rest);
            }
            // Children in source order: tuples, lists, dict entries (key, value), operators,
            // index and slice expressions.
            _ => e.visit_expr(|c| self.visit_expr(c)),
        }
    }

    // Reject f(*args) and f(**kwargs) calls.
    // Java: com/google/devtools/build/lib/packages/DotBazelFileSyntaxChecker.java:86 DotBazelFileSyntaxChecker.rejectStarArgs
    fn reject_star_args(&mut self, args: &[starlark::syntax::ast::AstArgument]) {
        for arg in args {
            match &arg.node {
                ArgumentP::KwArgs(value) => {
                    if !ALLOW_LITERAL_STAR_STAR_ARGS {
                        self.error(
                            arg.span,
                            format!(
                                "**kwargs arguments are not allowed in {WHERE}. Pass the arguments in explicitly."
                            ),
                        );
                    }
                    if !matches!(value.node, ExprP::Dict(_)) {
                        self.error(
                            arg.span,
                            format!("**kwargs arguments must be a literal dict in {WHERE}."),
                        );
                    }
                }
                ArgumentP::Args(_) => self.error(
                    arg.span,
                    format!(
                        "*args arguments are not allowed in {WHERE}. Pass the arguments in explicitly."
                    ),
                ),
                ArgumentP::Positional(_) | ArgumentP::Named(..) => {}
            }
        }
    }
}

/// The undefined-name part of Java's `Resolver` for a MODULE.bazel file that passed
/// [`check_module_file_syntax`] (no `def`, `lambda`, `for`, `if` or `load`).
// Java: net/starlark/java/syntax/Resolver.java:923 Resolver.use
fn resolve_names(ast: &AstModule, predeclared_env: &Globals) -> Vec<SyntaxError> {
    let stmts: Vec<&AstStmt> = match &ast.statement().node {
        StmtP::Statements(xs) => xs.iter().collect(),
        _ => vec![ast.statement()],
    };
    // Java: Resolver.createBindingsForBlock: every top-level binding is a global, visible to the
    // whole file.
    let mut globals: IndexSet<String> = IndexSet::new();
    for s in &stmts {
        match &s.node {
            StmtP::Assign(a) => a.lhs.visit_lvalue(|id| {
                globals.insert(id.node.ident.clone());
            }),
            StmtP::AssignModify(lhs, _, _) => lhs.visit_lvalue(|id| {
                globals.insert(id.node.ident.clone());
            }),
            _ => {}
        }
    }
    let mut r = NameResolver {
        ast,
        predeclared: predeclared_env.iter().map(|(n, _)| n.to_owned()).collect(),
        toplevel: globals,
        locals: Vec::new(),
        errors: Vec::new(),
    };
    for s in stmts {
        r.stmt(s);
    }
    r.errors
}

struct NameResolver<'a> {
    ast: &'a AstModule,
    /// Predeclared (MODULE.bazel natives) and universal names.
    predeclared: IndexSet<String>,
    /// Java `toplevel`: the file's globals, then predeclared names as they are first used.
    toplevel: IndexSet<String>,
    /// Comprehension blocks, innermost last.
    locals: Vec<IndexSet<String>>,
    errors: Vec<SyntaxError>,
}

impl NameResolver<'_> {
    fn stmt(&mut self, s: &AstStmt) {
        match &s.node {
            StmtP::Statements(xs) => {
                for x in xs {
                    self.stmt(x);
                }
            }
            StmtP::Expression(e) => self.expr(e),
            // Java: Resolver.visit(AssignmentStatement): the RHS, then the LHS.
            StmtP::Assign(a) => {
                self.expr(&a.rhs);
                self.target(&a.lhs);
            }
            StmtP::AssignModify(lhs, _, rhs) => {
                self.expr(rhs);
                self.target(lhs);
            }
            StmtP::Return(Some(e)) => self.expr(e),
            _ => {}
        }
    }

    /// The expressions inside an assignment target (identifiers there are bindings).
    fn target(&mut self, t: &AstAssignTarget) {
        match &t.node {
            AssignTargetP::Tuple(xs) => {
                for x in xs {
                    self.target(x);
                }
            }
            AssignTargetP::Index(b) => {
                self.expr(&b.0);
                self.expr(&b.1);
            }
            AssignTargetP::Dot(object, _) => self.expr(object),
            AssignTargetP::Identifier(_) => {}
        }
    }

    fn bind_for(target: &AstAssignTarget, block: &mut IndexSet<String>) {
        target.visit_lvalue(|id| {
            block.insert(id.node.ident.clone());
        });
    }

    // Java: net/starlark/java/syntax/Resolver.java Resolver.visit(Comprehension)
    fn comprehension(
        &mut self,
        bodies: &[&AstExpr],
        first: &ForClauseP<starlark::syntax::ast::AstNoPayload>,
        rest: &[ClauseP<starlark::syntax::ast::AstNoPayload>],
    ) {
        // Following Python3, the first for clause is resolved
        // outside the comprehension block. All the other loops
        // are resolved in the scope of their own bindings,
        // permitting forward references.
        self.expr(&first.over);
        let mut block = IndexSet::new();
        Self::bind_for(&first.var, &mut block);
        for clause in rest {
            if let ClauseP::For(f) = clause {
                Self::bind_for(&f.var, &mut block);
            }
        }
        self.locals.push(block);
        self.target(&first.var);
        for clause in rest {
            match clause {
                ClauseP::For(f) => {
                    self.expr(&f.over);
                    self.target(&f.var);
                }
                ClauseP::If(cond) => self.expr(cond),
            }
        }
        for body in bodies {
            self.expr(body);
        }
        self.locals.pop();
    }

    fn expr(&mut self, e: &AstExpr) {
        match &e.node {
            ExprP::Identifier(id) => self.use_(&id.node.ident, e.span),
            ExprP::ListComprehension(body, first, rest) => {
                self.comprehension(&[&**body], first, rest)
            }
            ExprP::DictComprehension(body, first, rest) => {
                self.comprehension(&[&body.0, &body.1], first, rest)
            }
            _ => e.visit_expr(|c| self.expr(c)),
        }
    }

    // Java: net/starlark/java/syntax/Resolver.java:923 Resolver.use
    fn use_(&mut self, name: &str, span: Span) {
        // Locally defined in a comprehension, or defined at toplevel?
        if self.locals.iter().any(|b| b.contains(name)) || self.toplevel.contains(name) {
            return;
        }
        if self.predeclared.contains(name) {
            self.toplevel.insert(name.to_owned());
            return;
        }
        // Java: Module.resolve throws Undefined with the candidates; Resolver adds the
        // spelling suggestion over getAllSymbols(candidates).
        let suggestion = SpellChecker::did_you_mean(name, self.all_symbols());
        let location = blaze_starlark::Location::of_span_start(&self.ast.file_span(span));
        self.errors.push(SyntaxError {
            location,
            message: format!("name '{name}' is not defined{suggestion}"),
        });
    }

    /// Java `getAllSymbols(candidates)`: a `HashSet` filled with the comprehension bindings
    /// (innermost block first), the module's candidates (a `HashSet` of predeclared and
    /// universal names) and the toplevel names.
    // PORT: java-hash-order. Insertion order within a hash bucket follows the order above, each
    // part in its own Java hash order; it only matters for equally good suggestions.
    // Java: net/starlark/java/syntax/Resolver.java:1266 Resolver.getAllSymbols
    fn all_symbols(&self) -> Vec<String> {
        let mut all: IndexSet<&str> = IndexSet::new();
        for block in self.locals.iter().rev() {
            all.extend(java_hash_order(block.iter().map(|s| s.as_str())));
        }
        all.extend(java_hash_order(self.predeclared.iter().map(|s| s.as_str())));
        all.extend(java_hash_order(self.toplevel.iter().map(|s| s.as_str())));
        java_hash_order(all.iter().copied())
            .into_iter()
            .map(|s| s.to_owned())
            .collect()
    }
}
