//! A stand-in for the Mockito `QueryEnvironment` mocks of the engine tests: only
//! `get_functions` answers; every other method fails loudly.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use blaze_cmdline::{Label, PackageIdentifier};
use blaze_query_engine::detailed_exit_code::DetailedExitCode;
use blaze_query_engine::min_depth_uniquifier::MinDepthUniquifier;
use blaze_query_engine::{
    Callback, EvaluateExpression, InterruptedException, QueryEnvironment, QueryError,
    QueryException, QueryExpression, QueryExpressionContext, QueryFunction, QueryTaskFuture,
    Setting, TargetAccessor, ThreadSafeMutableSet, TransitiveLoadFilesHelper, Uniquifier,
};

pub struct MockEnvironment {
    pub functions: Vec<Arc<dyn QueryFunction<()>>>,
}

impl QueryEnvironment<()> for MockEnvironment {
    fn get_sibling_targets_in_package(&self, _target: &()) -> Result<Vec<()>, QueryError> {
        unimplemented!("mock")
    }

    fn get_targets_matching_pattern(
        &self,
        _owner: &QueryExpression,
        _pattern: &str,
        _callback: &dyn Callback<()>,
    ) -> QueryTaskFuture<()> {
        unimplemented!("mock")
    }

    fn get_or_create(&self, _target: &()) {
        unimplemented!("mock")
    }

    fn get_fwd_deps(
        &self,
        _targets: &[()],
        _context: &QueryExpressionContext<()>,
    ) -> Result<Vec<()>, InterruptedException> {
        unimplemented!("mock")
    }

    fn get_reverse_deps(
        &self,
        _targets: &[()],
        _context: &QueryExpressionContext<()>,
    ) -> Result<Vec<()>, InterruptedException> {
        unimplemented!("mock")
    }

    fn get_transitive_closure(
        &self,
        _targets: &dyn ThreadSafeMutableSet<()>,
        _context: &QueryExpressionContext<()>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<()>>, InterruptedException> {
        unimplemented!("mock")
    }

    fn build_transitive_closure(
        &self,
        _caller: &QueryExpression,
        _target_nodes: &dyn ThreadSafeMutableSet<()>,
        _max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        unimplemented!("mock")
    }

    fn get_nodes_on_path(
        &self,
        _from: &(),
        _to: &(),
        _context: &QueryExpressionContext<()>,
    ) -> Result<Vec<()>, InterruptedException> {
        unimplemented!("mock")
    }

    fn eval(
        &self,
        _expr: &QueryExpression,
        _context: &QueryExpressionContext<()>,
        _callback: &dyn Callback<()>,
    ) -> QueryTaskFuture<()> {
        unimplemented!("mock")
    }

    fn create_evaluate_expression<'a>(
        &'a self,
        _expr: &QueryExpression,
        _context: &QueryExpressionContext<()>,
    ) -> Box<dyn EvaluateExpression<()> + 'a> {
        unimplemented!("mock")
    }

    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<()>> {
        unimplemented!("mock")
    }

    fn create_uniquifier(&self) -> Box<dyn Uniquifier<()>> {
        unimplemented!("mock")
    }

    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<()>> {
        unimplemented!("mock")
    }

    fn handle_error(
        &self,
        _expression: &QueryExpression,
        _message: &str,
        _detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        unimplemented!("mock")
    }

    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<()> + '_>, QueryException> {
        unimplemented!("mock")
    }

    fn transitive_load_files(
        &self,
        _targets: &[()],
        _also_add_build_files: bool,
        _seen_packages: &Mutex<HashSet<PackageIdentifier>>,
        _seen_bzl_labels: &Mutex<HashSet<Label>>,
        _uniquifier: &dyn Uniquifier<()>,
        _helper: &dyn TransitiveLoadFilesHelper<()>,
        _callback: &dyn Callback<()>,
    ) -> Result<(), QueryError> {
        unimplemented!("mock")
    }

    fn get_accessor(&self) -> &dyn TargetAccessor<()> {
        unimplemented!("mock")
    }

    fn is_setting_enabled(&self, _setting: Setting) -> bool {
        unimplemented!("mock")
    }

    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<()>>> {
        self.functions.clone()
    }
}
