//! Predeclared names of Bazel's ScriptTest harness, ported onto starlark-rust.
//!
//! Java: src/test/java/net/starlark/java/eval/ScriptTest.java (assert_, assert_eq, assert_fails,
//! struct, mutablestruct, freeze, int_mul_slow, json, _utf8_byte_strings).
//!
//! Per-chunk state (the Java `Reporter` thread-local) travels in `Evaluator::extra`.

use std::cell::RefCell;
use std::fmt;

use allocative::Allocative;
use blaze_starlark::structs::StructureMarker;
use num_bigint::BigInt;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::AllocValue;
use starlark::values::Demand;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;

/// What a reported assertion was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertKind {
    Assert,
    AssertEq,
    AssertFails,
}

/// Outcome category of one assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertOutcome {
    Pass,
    /// assert_ / assert_eq failed: values differ.
    ValueMismatch,
    /// assert_fails: an error happened, but its text did not match Bazel's regex.
    ErrorTextMismatch,
    /// assert_fails: no error at all where Bazel raises one.
    NoError,
}

/// One assertion record.
#[derive(Debug, Clone)]
pub struct AssertEvent {
    pub kind: AssertKind,
    pub outcome: AssertOutcome,
    /// 1-based line of the assertion call in the test file.
    pub line: usize,
    pub message: String,
}

/// The Java `ScriptTest.Reporter`, carried in `Evaluator::extra`.
#[derive(ProvidesStaticType, Default)]
pub struct Reporter {
    pub events: RefCell<Vec<AssertEvent>>,
}

impl Reporter {
    fn get<'a>(eval: &Evaluator<'_, 'a, '_>) -> &'a Reporter {
        eval.extra
            .and_then(|e| e.downcast_ref::<Reporter>())
            .expect("Evaluator::extra must hold a Reporter")
    }

    fn record(eval: &Evaluator, kind: AssertKind, outcome: AssertOutcome, message: String) {
        let line = eval
            .call_stack_top_location()
            .map(|l| l.resolve_span().begin.line + 1)
            .unwrap_or(0);
        Reporter::get(eval).events.borrow_mut().push(AssertEvent {
            kind,
            outcome,
            line,
            message,
        });
    }
}

/// Message of an evaluation error, without location or stack: Java `EvalException.getMessage()`.
pub fn bare_message(e: &starlark::Error) -> String {
    match e.kind() {
        // starlark-rust's `fail` prefixes a space: "fail: msg" -> "msg".
        starlark::ErrorKind::Fail(m) => m.to_string().trim_start().to_owned(),
        _ => format!("{:#}", e.without_diagnostic()),
    }
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

#[starlark_module]
pub fn script_test_builtins(builder: &mut GlobalsBuilder) {
    fn assert_<'v>(
        cond: Value<'v>,
        #[starlark(default = "assertion failed")] msg: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        if cond.to_bool() {
            Reporter::record(eval, AssertKind::Assert, AssertOutcome::Pass, String::new());
        } else {
            Reporter::record(
                eval,
                AssertKind::Assert,
                AssertOutcome::ValueMismatch,
                format!("assert_: {msg}"),
            );
        }
        Ok(NoneType)
    }

    fn assert_eq<'v>(
        x: Value<'v>,
        y: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // Java uses Object.equals; Starlark equality is the closest analogue.
        if x.equals(y)? {
            Reporter::record(
                eval,
                AssertKind::AssertEq,
                AssertOutcome::Pass,
                String::new(),
            );
        } else {
            let msg = match (x.unpack_str(), y.unpack_str()) {
                (Some(xs), Some(ys)) => format!(
                    "assert_eq: {} ({}) != {} ({})",
                    x.to_repr(),
                    hex(xs),
                    y.to_repr(),
                    hex(ys)
                ),
                _ => format!("assert_eq: {} != {}", x.to_repr(), y.to_repr()),
            };
            Reporter::record(
                eval,
                AssertKind::AssertEq,
                AssertOutcome::ValueMismatch,
                msg,
            );
        }
        Ok(NoneType)
    }

    fn assert_fails<'v>(
        #[starlark(require = pos)] f: Value<'v>,
        #[starlark(require = pos)] want_error: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let re = match regex::Regex::new(want_error) {
            Ok(re) => re,
            Err(_) => {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "invalid regexp: {want_error}"
                )));
            }
        };
        match eval.eval_function(f, &[], &[]) {
            Ok(_) => Reporter::record(
                eval,
                AssertKind::AssertFails,
                AssertOutcome::NoError,
                format!("evaluation succeeded unexpectedly (want error matching {want_error})"),
            ),
            Err(e) => {
                let msg = bare_message(&e);
                if re.is_match(&msg) {
                    Reporter::record(
                        eval,
                        AssertKind::AssertFails,
                        AssertOutcome::Pass,
                        String::new(),
                    );
                } else {
                    Reporter::record(
                        eval,
                        AssertKind::AssertFails,
                        AssertOutcome::ErrorTextMismatch,
                        format!("regular expression ({want_error}) did not match error ({msg})"),
                    );
                }
            }
        }
        Ok(NoneType)
    }

    fn r#struct<'v>(args: &Arguments<'v, '_>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        args.no_positional_args(heap)?;
        let fields = args
            .names_map()?
            .into_iter()
            .map(|(k, v)| (k.as_str().to_owned(), v))
            .collect();
        Ok(heap.alloc(TestStruct { fields }))
    }

    fn mutablestruct<'v>(args: &Arguments<'v, '_>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        args.no_positional_args(heap)?;
        let fields = args
            .names_map()?
            .into_iter()
            .map(|(k, v)| (k.as_str().to_owned(), v))
            .collect();
        Ok(heap.alloc(MutableStruct {
            fields: RefCell::new(fields),
        }))
    }

    /// Java shallow-freezes a value or closes the thread's Mutability. starlark-rust can only
    /// freeze a whole Module after evaluation, so this is reported as a harness limitation.
    fn freeze<'v>(#[starlark(args)] _args: UnpackTuple<Value<'v>>) -> starlark::Result<NoneType> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "HARNESS: freeze() is not expressible on starlark-rust (no shallow/thread freeze)"
        )))
    }

    fn int_mul_slow<'v>(x: BigInt, y: BigInt, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        Ok(heap.alloc(x * y))
    }

    const _utf8_byte_strings: bool = false;
}

/// `print` for the harness: ScriptTest output is not checked.
#[starlark_module]
pub fn discard_print(builder: &mut GlobalsBuilder) {
    fn print<'v>(
        #[starlark(args)] _args: UnpackTuple<Value<'v>>,
        #[starlark(require = named, default = " ")] _sep: &str,
    ) -> starlark::Result<NoneType> {
        Ok(NoneType)
    }
}

/// ScriptTest's `struct` (Java `ScriptTest.ImmutableStruct`), repr `struct(a = 1)`.
#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct TestStruct<'v> {
    fields: Vec<(String, Value<'v>)>,
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenTestStruct {
    fields: Vec<(String, FrozenValue)>,
}

/// ScriptTest's `mutablestruct` (Java `ScriptTest.MutableStruct`). Never frozen by the harness,
/// so it is allocated with `alloc_complex_no_freeze`.
#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct MutableStruct<'v> {
    fields: RefCell<Vec<(String, Value<'v>)>>,
}

impl<'v> AllocValue<'v> for TestStruct<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> AllocValue<'v> for MutableStruct<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl<'v> Freeze for TestStruct<'v> {
    type Frozen = FrozenTestStruct;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenTestStruct> {
        Ok(FrozenTestStruct {
            fields: self.fields.freeze(freezer)?,
        })
    }
}

fn fmt_struct(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    fields: impl Iterator<Item = (String, String)>,
) -> fmt::Result {
    write!(f, "{name}(")?;
    for (i, (k, v)) in fields.enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{k} = {v}")?;
    }
    write!(f, ")")
}

impl fmt::Display for TestStruct<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_struct(
            f,
            "struct",
            self.fields.iter().map(|(k, v)| (k.clone(), v.to_repr())),
        )
    }
}

impl fmt::Display for FrozenTestStruct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_struct(
            f,
            "struct",
            self.fields
                .iter()
                .map(|(k, v)| (k.clone(), v.to_value().to_repr())),
        )
    }
}

impl fmt::Display for MutableStruct<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_struct(
            f,
            "mutablestruct",
            self.fields
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), v.to_repr())),
        )
    }
}

fn no_field_assignment() -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!(
        "struct value does not support field assignment"
    ))
}

#[starlark_value(type = "struct")]
impl<'v> StarlarkValue<'v> for TestStruct<'v> {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.fields
            .iter()
            .find(|(k, _)| k == attribute)
            .map(|(_, v)| *v)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.fields.iter().map(|(k, _)| k.clone()).collect()
    }

    // Java: ScriptTest.Struct implements Structure (json.encode encodes it as an object).
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(StructureMarker { native_info: false });
    }

    fn set_attr(&self, _attribute: &str, _new_value: Value<'v>) -> starlark::Result<()> {
        Err(no_field_assignment())
    }
}

#[starlark_value(type = "struct")]
impl<'v> StarlarkValue<'v> for FrozenTestStruct {
    type Canonical = TestStruct<'v>;

    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.fields
            .iter()
            .find(|(k, _)| k == attribute)
            .map(|(_, v)| v.to_value())
    }

    fn dir_attr(&self) -> Vec<String> {
        self.fields.iter().map(|(k, _)| k.clone()).collect()
    }

    // Java: ScriptTest.Struct implements Structure (json.encode encodes it as an object).
    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(StructureMarker { native_info: false });
    }

    fn set_attr(&self, _attribute: &str, _new_value: Value<'v>) -> starlark::Result<()> {
        Err(no_field_assignment())
    }
}

#[starlark_value(type = "mutablestruct")]
impl<'v> StarlarkValue<'v> for MutableStruct<'v> {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.fields
            .borrow()
            .iter()
            .find(|(k, _)| k == attribute)
            .map(|(_, v)| *v)
    }

    fn dir_attr(&self) -> Vec<String> {
        self.fields
            .borrow()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn set_attr(&self, attribute: &str, new_value: Value<'v>) -> starlark::Result<()> {
        if new_value.unpack_str() == Some("bad") {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "bad field value"
            )));
        }
        let mut fields = self.fields.borrow_mut();
        match fields.iter_mut().find(|(k, _)| k == attribute) {
            Some(slot) => slot.1 = new_value,
            None => fields.push((attribute.to_owned(), new_value)),
        }
        Ok(())
    }
}
