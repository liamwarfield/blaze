//! Port of `query2/engine/AggregatingQueryExpressionVisitor.java`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::query_environment::ArgumentType;
use crate::query_expression::QueryExpression;
use crate::query_expression_visitor::QueryExpressionVisitor;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// The results of all direct sub-expression visitations, keyed by sub-expression identity.
///
/// PORT: Java builds `ImmutableMap.copyOf(HashMap<QueryExpression, T>)` keyed by reference
/// identity, whose iteration order follows identity hash codes (nondeterministic). This list is in
/// visitation order, one entry per distinct sub-expression (a repeated sub-expression keeps its
/// first position and its last result, as `HashMap.put` would).
pub type AggregateResultMap<R> = Vec<(QueryExpression, R)>;

fn put<R>(map: &mut AggregateResultMap<R>, expr: QueryExpression, value: R) {
    match map.iter_mut().find(|(e, _)| e.ptr_eq(&expr)) {
        Some(entry) => entry.1 = value,
        None => map.push((expr, value)),
    }
}

/// An implementation of `QueryExpressionVisitor` which recursively visits all nested
/// `QueryExpression`s.
///
/// PORT: implementors write their `QueryExpressionVisitor` methods by delegating to the provided
/// `aggregating_visit_*` methods (Java's inherited implementations, also what `super.visit`
/// calls), and implement `aggregate` and `visit_target_literal`.
// Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:29 AggregatingQueryExpressionVisitor
pub trait AggregatingQueryExpressionVisitor<R, C>: QueryExpressionVisitor<R, C> + Sized {
    /// Aggregates results from all sub-expression visitations. The map is guaranteed to include
    /// results for all direct sub-expressions.
    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:76 AggregatingQueryExpressionVisitor.aggregate
    fn aggregate(&mut self, result_map: AggregateResultMap<R>) -> R;

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:33 AggregatingQueryExpressionVisitor.visit
    fn aggregating_visit_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &C,
    ) -> R {
        let mut query_expression_mapping = Vec::new();
        for expr in binary_operator_expression.get_operands() {
            let value = expr.accept(self, context);
            put(&mut query_expression_mapping, expr.clone(), value);
        }
        self.aggregate(query_expression_mapping)
    }

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:42 AggregatingQueryExpressionVisitor.visit
    fn aggregating_visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &C,
    ) -> R {
        let mut query_expression_mapping = Vec::new();
        for argument in function_expression.get_args() {
            if argument.get_type() == ArgumentType::Expression {
                let value = argument.get_expression().accept(self, context);
                put(
                    &mut query_expression_mapping,
                    argument.get_expression().clone(),
                    value,
                );
            }
        }

        self.aggregate(query_expression_mapping)
    }

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:55 AggregatingQueryExpressionVisitor.visit
    fn aggregating_visit_let_expression(
        &mut self,
        let_expression: &Arc<LetExpression>,
        context: &C,
    ) -> R {
        // ImmutableMap.of(k1, v1, k2, v2) evaluates both values before building the map.
        let var_value = let_expression.get_var_expr().accept(self, context);
        let body_value = let_expression.get_body_expr().accept(self, context);
        if let_expression
            .get_var_expr()
            .ptr_eq(let_expression.get_body_expr())
        {
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                let_expression.get_var_expr()
            );
        }
        self.aggregate(vec![
            (let_expression.get_var_expr().clone(), var_value),
            (let_expression.get_body_expr().clone(), body_value),
        ])
    }

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:63 AggregatingQueryExpressionVisitor.visit
    fn aggregating_visit_set_expression(
        &mut self,
        set_expression: &Arc<SetExpression>,
        context: &C,
    ) -> R {
        let mut query_expression_mapping = Vec::new();
        for target_literal in set_expression.get_words() {
            let expr = QueryExpression::TargetLiteral(target_literal.clone());
            let value = self.visit_target_literal(target_literal, context);
            put(&mut query_expression_mapping, expr, value);
        }

        self.aggregate(query_expression_mapping)
    }
}

/// Returns `true` when the query expression contains at least one `QueryFunction` whose name is
/// in the set of `function_names`.
// Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:82 AggregatingQueryExpressionVisitor.ContainsFunctionQueryExpressionVisitor
pub struct ContainsFunctionQueryExpressionVisitor {
    function_names: HashSet<String>,
}

impl ContainsFunctionQueryExpressionVisitor {
    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:88 ContainsFunctionQueryExpressionVisitor.<init>
    pub fn new(function_names: impl IntoIterator<Item = String>) -> Self {
        ContainsFunctionQueryExpressionVisitor {
            function_names: function_names.into_iter().collect(),
        }
    }
}

impl QueryExpressionVisitor<bool, ()> for ContainsFunctionQueryExpressionVisitor {
    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:93 ContainsFunctionQueryExpressionVisitor.visit
    fn visit_target_literal(
        &mut self,
        _target_literal: &Arc<TargetLiteral>,
        _context: &(),
    ) -> bool {
        false
    }

    fn visit_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &(),
    ) -> bool {
        self.aggregating_visit_binary_operator_expression(binary_operator_expression, context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:103 ContainsFunctionQueryExpressionVisitor.visit
    fn visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &(),
    ) -> bool {
        let function = function_expression.get_function();
        if self.function_names.contains(&function.get_name()) {
            true
        } else {
            self.aggregating_visit_function_expression(function_expression, context)
        }
    }

    fn visit_let_expression(&mut self, let_expression: &Arc<LetExpression>, context: &()) -> bool {
        self.aggregating_visit_let_expression(let_expression, context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:98 ContainsFunctionQueryExpressionVisitor.visit
    fn visit_set_expression(
        &mut self,
        _set_expression: &Arc<SetExpression>,
        _context: &(),
    ) -> bool {
        false
    }
}

impl AggregatingQueryExpressionVisitor<bool, ()> for ContainsFunctionQueryExpressionVisitor {
    // Java: com/google/devtools/build/lib/query2/engine/AggregatingQueryExpressionVisitor.java:113 ContainsFunctionQueryExpressionVisitor.aggregate
    fn aggregate(&mut self, result_map: AggregateResultMap<bool>) -> bool {
        result_map.iter().any(|(_, v)| *v)
    }
}
