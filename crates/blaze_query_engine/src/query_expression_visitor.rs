//! Port of `query2/engine/QueryExpressionVisitor.java`.

use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// Provides interfaces to visit all `QueryExpression`s with a context object.
///
/// PORT: Java's overloaded `visit` methods are named after the node class. Each receives the
/// shared node, so a mapper can return the same node (Java reference equality).
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionVisitor.java:17 QueryExpressionVisitor
pub trait QueryExpressionVisitor<R, C> {
    fn visit_target_literal(&mut self, target_literal: &Arc<TargetLiteral>, context: &C) -> R;

    fn visit_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &C,
    ) -> R;

    fn visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &C,
    ) -> R;

    fn visit_let_expression(&mut self, let_expression: &Arc<LetExpression>, context: &C) -> R;

    fn visit_set_expression(&mut self, set_expression: &Arc<SetExpression>, context: &C) -> R;
}
