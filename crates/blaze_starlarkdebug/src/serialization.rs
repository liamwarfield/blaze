//! Port of `DebuggerSerialization` and `ThreadObjectMap`: Starlark values as the debugger's
//! `Value` protos, and the paused thread's table of values with children.

use blaze_protos::starlark_debugging::Value as ValueProto;
use blaze_starlark::structs::StructureMarker;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::tuple::TupleRef;
use starlark::values::{Heap, Value};

/// An object the debugger can ask the children of: a Starlark value, or an entry of a dict
/// (Java `Map.Entry`).
#[derive(Clone, Copy)]
pub enum DebugObject<'v> {
    Value(Value<'v>),
    MapEntry(Value<'v>, Value<'v>),
}

/// A map of value IDs to their values, for the values a paused thread has shown the client.
/// Lives as long as the pause.
// Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadObjectMap.java:32 ThreadObjectMap
#[derive(Default)]
pub struct ThreadObjectMap<'v> {
    objects: Vec<DebugObject<'v>>,
}

impl<'v> ThreadObjectMap<'v> {
    /// Registers a value and returns its ID (IDs start at 1).
    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadObjectMap.java:43 ThreadObjectMap.registerValue
    pub fn register_value(&mut self, value: DebugObject<'v>) -> i64 {
        self.objects.push(value);
        self.objects.len() as i64
    }

    // Java: com/google/devtools/build/lib/starlarkdebug/server/ThreadObjectMap.java:51 ThreadObjectMap.getValue
    pub fn get_value(&self, identifier: i64) -> Option<DebugObject<'v>> {
        usize::try_from(identifier)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| self.objects.get(i).copied())
    }
}

/// The Java class of an entry of a `Dict` (`LinkedHashMap.entrySet()`), which the debugger
/// shows by its class.
const MAP_ENTRY_CLASS: &str = "java.util.LinkedHashMap$Entry";

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:36 DebuggerSerialization.getValueProto
pub fn get_value_proto<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    label: &str,
    value: DebugObject<'v>,
) -> ValueProto {
    // TODO(bazel-team): prune cycles, and provide a way to limit breadth/depth of children
    // reported.
    let has_children = has_children(value);
    ValueProto {
        label: label.to_owned(),
        // TODO(bazel-team): omit type details for non-Starlark values
        r#type: type_of(value),
        description: get_description(value),
        has_children,
        id: if has_children {
            object_map.register_value(value)
        } else {
            0
        },
    }
}

/// `Starlark.type(value)`.
fn type_of(value: DebugObject) -> String {
    match value {
        DebugObject::Value(v) => v.get_type().to_owned(),
        // Starlark.classType of a non-Starlark class: its simple name.
        DebugObject::MapEntry(..) => "Entry".to_owned(),
    }
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:49 DebuggerSerialization.getDescription
fn get_description(value: DebugObject) -> String {
    match value {
        DebugObject::Value(v) => match v.unpack_str() {
            Some(s) => s.to_owned(),
            None => v.to_repr(),
        },
        // Printer.repr of a non-Starlark object.
        DebugObject::MapEntry(..) => format!("<unknown object {MAP_ENTRY_CLASS}>"),
    }
}

/// The elements of an `Iterable` value (list, tuple, set, range), or `None`.
fn iterable_elements<'v>(v: Value<'v>, heap: Heap<'v>) -> Option<Vec<Value<'v>>> {
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.content().to_vec());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Some(t.content().to_vec());
    }
    if matches!(v.get_type(), "set" | "range") {
        return v.iterate(heap).ok().map(|it| it.collect());
    }
    None
}

/// Whether the value is a Java `Iterable` that is empty (`set` and `range` checked by length).
fn is_empty_iterable(v: Value) -> Option<bool> {
    if let Some(l) = ListRef::from_value(v) {
        return Some(l.is_empty());
    }
    if let Some(t) = TupleRef::from_value(v) {
        return Some(t.len() == 0);
    }
    if matches!(v.get_type(), "set" | "range") {
        return v.length().ok().map(|n| n == 0);
    }
    None
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:56 DebuggerSerialization.hasChildren
fn has_children(value: DebugObject) -> bool {
    let v = match value {
        DebugObject::MapEntry(..) => return true,
        DebugObject::Value(v) => v,
    };
    if let Some(d) = DictRef::from_value(v) {
        return !d.is_empty();
    }
    if let Some(empty) = is_empty_iterable(v) {
        return !empty;
    }
    // String and Boolean are no StarlarkValues; StarlarkInt has no children; every other value
    // (None, float, functions, structs, ...) is a StarlarkValue.
    !matches!(v.get_type(), "string" | "bool" | "int")
}

// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:84 DebuggerSerialization.getChildren
pub fn get_children<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    value: DebugObject<'v>,
    heap: Heap<'v>,
) -> Vec<ValueProto> {
    let v = match value {
        // Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:152 DebuggerSerialization.getChildren
        DebugObject::MapEntry(key, value) => {
            return vec![
                get_value_proto(object_map, "key", DebugObject::Value(key)),
                get_value_proto(object_map, "value", DebugObject::Value(value)),
            ];
        }
        DebugObject::Value(v) => v,
    };
    if let Some(d) = DictRef::from_value(v) {
        // getChildren(objectMap, map.entrySet()): an Iterable of entries.
        let entries: Vec<(Value<'v>, Value<'v>)> = d.iter().collect();
        drop(d);
        return entries
            .into_iter()
            .enumerate()
            .map(|(i, (k, val))| {
                get_value_proto(object_map, &format!("[{i}]"), DebugObject::MapEntry(k, val))
            })
            .collect();
    }
    if let Some(elements) = iterable_elements(v, heap) {
        // Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:159 DebuggerSerialization.getChildren
        return elements
            .into_iter()
            .enumerate()
            .map(|(i, e)| get_value_proto(object_map, &format!("[{i}]"), DebugObject::Value(e)))
            .collect();
    }
    if matches!(v.get_type(), "string" | "bool" | "int") {
        return Vec::new();
    }
    if v.request_value::<StructureMarker>().is_some() {
        return get_structure_children(object_map, v, heap);
    }
    get_starlark_value_children(object_map, v, heap)
}

/// The fields of a `Structure`, sorted, without its methods.
// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:111 DebuggerSerialization.getChildren
fn get_structure_children<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    v: Value<'v>,
    heap: Heap<'v>,
) -> Vec<ValueProto> {
    let mut names: Vec<String> = v
        .dir_attr()
        .into_iter()
        .filter(|name| v.has_attr(name, heap))
        .collect();
    names.sort();
    let mut children = Vec::new();
    for key in names {
        // getFieldNames() lists the fields only: an attribute that is a method of the value
        // (struct's to_json, to_proto) is not one of them.
        if let Some(value) = v.get_attr(&key, heap).ok().flatten() {
            if value.get_type() == "builtin_function_or_method" {
                continue;
            }
            children.push(get_value_proto(object_map, &key, DebugObject::Value(value)));
        }
    }
    children
}

/// `Starlark.dir(value)` and `Starlark.getattr` of each name (methods included).
// Java: com/google/devtools/build/lib/starlarkdebug/server/DebuggerSerialization.java:127 DebuggerSerialization.getChildren
fn get_starlark_value_children<'v>(
    object_map: &mut ThreadObjectMap<'v>,
    v: Value<'v>,
    heap: Heap<'v>,
) -> Vec<ValueProto> {
    let mut names = v.dir_attr();
    names.sort();
    let mut children = Vec::new();
    for name in names {
        // getattr errors are ignored (Java catches EvalException).
        if let Some(value) = v.get_attr(&name, heap).ok().flatten() {
            children.push(get_value_proto(
                object_map,
                &name,
                DebugObject::Value(value),
            ));
        }
    }
    children
}
