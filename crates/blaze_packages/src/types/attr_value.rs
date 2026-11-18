//! Converted attribute values as plain Rust data (crates/blaze_packages/DESIGN.md "Types").
//!
//! Java stores converted attribute values as `Object` (StarlarkInt, String, Boolean, Label,
//! ImmutableList, ImmutableMap, License, TriState) or as the special objects Bazel puts in
//! attribute slots (`BuildType.SelectorList`, `Attribute.ComputedDefault`,
//! `Attribute.StarlarkComputedDefaultTemplate`, `Attribute.LateBoundDefault`,
//! `MaterializingDefault`). [`AttrValue`] is that closed set.

use std::sync::Arc;

use blaze_cmdline::Label;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::attribute::ComputedDefault;
use crate::attribute::LateBoundDefault;
use crate::attribute::MaterializingDefault;
use crate::attribute::StarlarkComputedDefaultTemplate;
use crate::types::build_type::BuildTypeSelectorList;
use crate::types::license::License;
use crate::types::tri_state::TriState;

/// A converted attribute value (Java `Object` held in an attribute slot).
#[derive(Debug, Clone)]
pub enum AttrValue {
    /// `Type.INTEGER` (a StarlarkInt in the signed 32-bit range).
    Int(i32),
    /// `Type.STRING`, `Type.STRING_NO_INTERN`.
    String(String),
    /// `Type.BOOLEAN`.
    Bool(bool),
    /// `BuildType.LABEL`, `NODEP_LABEL`, `DORMANT_LABEL`, `GENQUERY_SCOPE_TYPE`, `OUTPUT`.
    Label(Label),
    /// `Types.STRING_LIST`.
    StringList(Vec<String>),
    /// `Types.STRING_SET`.
    StringSet(IndexSet<String>),
    /// `Types.INTEGER_LIST`.
    IntList(Vec<i32>),
    /// `BuildType.LABEL_LIST`, `NODEP_LABEL_LIST`, `DORMANT_LABEL_LIST`,
    /// `GENQUERY_SCOPE_TYPE_LIST`, `OUTPUT_LIST`.
    LabelList(Vec<Label>),
    /// `Types.STRING_DICT`.
    StringDict(IndexMap<String, String>),
    /// `Types.STRING_LIST_DICT`.
    StringListDict(IndexMap<String, Vec<String>>),
    /// `BuildType.LABEL_DICT_UNARY`.
    LabelDictUnary(IndexMap<String, Label>),
    /// `BuildType.LABEL_KEYED_STRING_DICT`.
    LabelKeyedStringDict(IndexMap<Label, String>),
    /// `BuildType.LABEL_LIST_DICT`.
    LabelListDict(IndexMap<String, Vec<Label>>),
    /// `BuildType.LICENSE`.
    License(License),
    /// `BuildType.TRISTATE`.
    TriState(TriState),
    /// A configurable value (Java `BuildType.SelectorList`).
    SelectorList(Arc<BuildTypeSelectorList>),
    /// Java `Attribute.ComputedDefault` (native or precomputed Starlark).
    ComputedDefault(Arc<ComputedDefault>),
    /// Java `Attribute.StarlarkComputedDefaultTemplate` (an attribute's default before a rule
    /// instance precomputes it).
    StarlarkComputedDefaultTemplate(Arc<StarlarkComputedDefaultTemplate>),
    /// Java `Attribute.LateBoundDefault`.
    LateBound(Arc<LateBoundDefault>),
    /// Java `MaterializingDefault`.
    Materializer(Arc<MaterializingDefault>),
}

impl PartialEq for AttrValue {
    // Java `Object.equals` of the stored values: value equality for data, identity for the
    // default-computing objects (they do not override equals).
    fn eq(&self, other: &AttrValue) -> bool {
        match (self, other) {
            (AttrValue::Int(a), AttrValue::Int(b)) => a == b,
            (AttrValue::String(a), AttrValue::String(b)) => a == b,
            (AttrValue::Bool(a), AttrValue::Bool(b)) => a == b,
            (AttrValue::Label(a), AttrValue::Label(b)) => a == b,
            (AttrValue::StringList(a), AttrValue::StringList(b)) => a == b,
            (AttrValue::StringSet(a), AttrValue::StringSet(b)) => a == b,
            (AttrValue::IntList(a), AttrValue::IntList(b)) => a == b,
            (AttrValue::LabelList(a), AttrValue::LabelList(b)) => a == b,
            (AttrValue::StringDict(a), AttrValue::StringDict(b)) => a == b,
            (AttrValue::StringListDict(a), AttrValue::StringListDict(b)) => a == b,
            (AttrValue::LabelDictUnary(a), AttrValue::LabelDictUnary(b)) => a == b,
            (AttrValue::LabelKeyedStringDict(a), AttrValue::LabelKeyedStringDict(b)) => a == b,
            (AttrValue::LabelListDict(a), AttrValue::LabelListDict(b)) => a == b,
            (AttrValue::License(a), AttrValue::License(b)) => a == b,
            (AttrValue::TriState(a), AttrValue::TriState(b)) => a == b,
            (AttrValue::SelectorList(a), AttrValue::SelectorList(b)) => a == b,
            (AttrValue::ComputedDefault(a), AttrValue::ComputedDefault(b)) => Arc::ptr_eq(a, b),
            (
                AttrValue::StarlarkComputedDefaultTemplate(a),
                AttrValue::StarlarkComputedDefaultTemplate(b),
            ) => Arc::ptr_eq(a, b),
            (AttrValue::LateBound(a), AttrValue::LateBound(b)) => Arc::ptr_eq(a, b),
            (AttrValue::Materializer(a), AttrValue::Materializer(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl AttrValue {
    pub fn as_int(&self) -> Option<i32> {
        match self {
            AttrValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            AttrValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            AttrValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_label(&self) -> Option<&Label> {
        match self {
            AttrValue::Label(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_string_list(&self) -> Option<&[String]> {
        match self {
            AttrValue::StringList(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_label_list(&self) -> Option<&[Label]> {
        match self {
            AttrValue::LabelList(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_string_dict(&self) -> Option<&IndexMap<String, String>> {
        match self {
            AttrValue::StringDict(d) => Some(d),
            _ => None,
        }
    }

    pub fn as_license(&self) -> Option<&License> {
        match self {
            AttrValue::License(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_tri_state(&self) -> Option<TriState> {
        match self {
            AttrValue::TriState(t) => Some(*t),
            _ => None,
        }
    }

    pub fn as_selector_list(&self) -> Option<&Arc<BuildTypeSelectorList>> {
        match self {
            AttrValue::SelectorList(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_computed_default(&self) -> Option<&Arc<ComputedDefault>> {
        match self {
            AttrValue::ComputedDefault(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_late_bound(&self) -> Option<&Arc<LateBoundDefault>> {
        match self {
            AttrValue::LateBound(l) => Some(l),
            _ => None,
        }
    }

    /// Java `instanceof SelectorList`.
    pub fn is_selector_list(&self) -> bool {
        matches!(self, AttrValue::SelectorList(_))
    }

    /// Java `instanceof ComputedDefault`.
    pub fn is_computed_default(&self) -> bool {
        matches!(self, AttrValue::ComputedDefault(_))
    }

    /// Java `instanceof LateBoundDefault`.
    pub fn is_late_bound(&self) -> bool {
        matches!(self, AttrValue::LateBound(_))
    }
}
