//! Port of `query2/engine/TotalWeightQueryExpressionVisitor.java`.

use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::query_environment::ArgumentType;
use crate::query_expression_visitor::QueryExpressionVisitor;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// Java `String.length()` of a string in Bazel's internal form (one char per UTF-8 byte; the
/// query text reaches the parser in internal form).
///
/// PORT: StringEncoding identity.
fn java_length(s: &str) -> i64 {
    s.len() as i64
}

/// A `QueryExpressionVisitor` that cheaply estimates the size of a `QueryExpression` without
/// stringify-ing it.
// Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:22 TotalWeightQueryExpressionVisitor
#[derive(Debug, Default)]
pub struct TotalWeightQueryExpressionVisitor;

impl TotalWeightQueryExpressionVisitor {
    pub fn new() -> Self {
        TotalWeightQueryExpressionVisitor
    }
}

impl QueryExpressionVisitor<i64, ()> for TotalWeightQueryExpressionVisitor {
    // Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:25 TotalWeightQueryExpressionVisitor.visit
    fn visit_target_literal(&mut self, target_literal: &Arc<TargetLiteral>, _context: &()) -> i64 {
        java_length(target_literal.get_pattern())
    }

    // Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:30 TotalWeightQueryExpressionVisitor.visit
    fn visit_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        _context: &(),
    ) -> i64 {
        let mut total_weight = 0i64;
        for operand in binary_operator_expression.get_operands() {
            total_weight += operand.accept(self, &());
        }
        total_weight
    }

    // Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:39 TotalWeightQueryExpressionVisitor.visit
    fn visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        _context: &(),
    ) -> i64 {
        let mut total_weight = 0i64;
        for arg in function_expression.get_args() {
            total_weight += match arg.get_type() {
                ArgumentType::Word => java_length(arg.get_word()),
                ArgumentType::Integer => 1,
                ArgumentType::Expression => arg.get_expression().accept(self, &()),
            };
        }
        total_weight
    }

    // Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:53 TotalWeightQueryExpressionVisitor.visit
    fn visit_let_expression(&mut self, let_expression: &Arc<LetExpression>, _context: &()) -> i64 {
        java_length(let_expression.get_var_name())
            + let_expression.get_var_expr().accept(self, &())
            + let_expression.get_body_expr().accept(self, &())
    }

    // Java: com/google/devtools/build/lib/query2/engine/TotalWeightQueryExpressionVisitor.java:60 TotalWeightQueryExpressionVisitor.visit
    fn visit_set_expression(&mut self, set_expression: &Arc<SetExpression>, _context: &()) -> i64 {
        let mut total_weight = 0i64;
        for word in set_expression.get_words() {
            total_weight += java_length(word.get_pattern());
        }
        total_weight
    }
}
