//! Port of `query2/engine/QueryExpressionMapper.java`.

use std::sync::Arc;

use crate::binary_operator_expression::BinaryOperatorExpression;
use crate::function_expression::FunctionExpression;
use crate::let_expression::LetExpression;
use crate::query_environment::{Argument, ArgumentType};
use crate::query_expression::QueryExpression;
use crate::query_expression_visitor::QueryExpressionVisitor;
use crate::set_expression::SetExpression;
use crate::target_literal::TargetLiteral;

/// Performs an arbitrary contextual transformation of a `QueryExpression`.
///
/// For each subclass of `QueryExpression`, there's a corresponding `visit_*` method that
/// transforms a node of that type. By default, this method recursively applies this
/// `QueryExpressionMapper`'s transformation in a structure-preserving manner (trying to maintain
/// reference-equality, as an optimization). Implementors can override these methods in order to
/// implement an arbitrary transformation.
///
/// PORT: every `QueryExpressionMapper<C>` is a `QueryExpressionVisitor<QueryExpression, C>`
/// (blanket impl). Java's `super.visit(...)` from an override is the matching free function
/// (`default_visit_binary_operator_expression`, ...).
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:28 QueryExpressionMapper
pub trait QueryExpressionMapper<C> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:32 QueryExpressionMapper.visit
    fn map_target_literal(
        &mut self,
        target_literal: &Arc<TargetLiteral>,
        _context: &C,
    ) -> QueryExpression {
        QueryExpression::TargetLiteral(target_literal.clone())
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:37 QueryExpressionMapper.visit
    fn map_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &C,
    ) -> QueryExpression
    where
        Self: Sized,
    {
        default_visit_binary_operator_expression(self, binary_operator_expression, context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:54 QueryExpressionMapper.visit
    fn map_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &C,
    ) -> QueryExpression
    where
        Self: Sized,
    {
        default_visit_function_expression(self, function_expression, context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:76 QueryExpressionMapper.visit
    fn map_let_expression(
        &mut self,
        let_expression: &Arc<LetExpression>,
        context: &C,
    ) -> QueryExpression
    where
        Self: Sized,
    {
        default_visit_let_expression(self, let_expression, context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:92 QueryExpressionMapper.visit
    fn map_set_expression(
        &mut self,
        set_expression: &Arc<SetExpression>,
        _context: &C,
    ) -> QueryExpression {
        QueryExpression::Set(set_expression.clone())
    }
}

impl<C, M: QueryExpressionMapper<C>> QueryExpressionVisitor<QueryExpression, C> for M {
    fn visit_target_literal(
        &mut self,
        target_literal: &Arc<TargetLiteral>,
        context: &C,
    ) -> QueryExpression {
        self.map_target_literal(target_literal, context)
    }

    fn visit_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_binary_operator_expression(binary_operator_expression, context)
    }

    fn visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_function_expression(function_expression, context)
    }

    fn visit_let_expression(
        &mut self,
        let_expression: &Arc<LetExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_let_expression(let_expression, context)
    }

    fn visit_set_expression(
        &mut self,
        set_expression: &Arc<SetExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_set_expression(set_expression, context)
    }
}

/// Java `QueryExpressionMapper.visit(BinaryOperatorExpression, C)`.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:37 QueryExpressionMapper.visit
pub fn default_visit_binary_operator_expression<C>(
    mapper: &mut (impl QueryExpressionVisitor<QueryExpression, C> + ?Sized),
    binary_operator_expression: &Arc<BinaryOperatorExpression>,
    context: &C,
) -> QueryExpression {
    let mut changed = false;
    let mut mapped_operands_builder = Vec::new();
    for operand in binary_operator_expression.get_operands() {
        let mapped_operand = operand.accept(mapper, context);
        if !mapped_operand.ptr_eq(operand) {
            changed = true;
        }
        mapped_operands_builder.push(mapped_operand);
    }
    if changed {
        BinaryOperatorExpression::new(
            binary_operator_expression.get_operator(),
            mapped_operands_builder,
        )
        .into()
    } else {
        QueryExpression::BinaryOperator(binary_operator_expression.clone())
    }
}

/// Java `QueryExpressionMapper.visit(FunctionExpression, C)`.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:54 QueryExpressionMapper.visit
pub fn default_visit_function_expression<C>(
    mapper: &mut (impl QueryExpressionVisitor<QueryExpression, C> + ?Sized),
    function_expression: &Arc<FunctionExpression>,
    context: &C,
) -> QueryExpression {
    let mut changed = false;
    let mut mapped_argument_builder = Vec::new();
    for argument in function_expression.get_args() {
        match argument.get_type() {
            ArgumentType::Expression => {
                let expr = argument.get_expression();
                let mapped_expression = expr.accept(mapper, context);
                if !expr.ptr_eq(&mapped_expression) {
                    changed = true;
                }
                mapped_argument_builder.push(Argument::of_expression(mapped_expression));
            }
            _ => mapped_argument_builder.push(argument.clone()),
        }
    }
    if changed {
        function_expression
            .with_args(mapped_argument_builder)
            .into()
    } else {
        QueryExpression::Function(function_expression.clone())
    }
}

/// Java `QueryExpressionMapper.visit(LetExpression, C)`.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:76 QueryExpressionMapper.visit
pub fn default_visit_let_expression<C>(
    mapper: &mut (impl QueryExpressionVisitor<QueryExpression, C> + ?Sized),
    let_expression: &Arc<LetExpression>,
    context: &C,
) -> QueryExpression {
    let mut changed = false;
    let mapped_var_expr = let_expression.get_var_expr().accept(mapper, context);
    if !mapped_var_expr.ptr_eq(let_expression.get_var_expr()) {
        changed = true;
    }
    let mapped_body_expr = let_expression.get_body_expr().accept(mapper, context);
    if !mapped_body_expr.ptr_eq(let_expression.get_body_expr()) {
        changed = true;
    }
    if changed {
        LetExpression::new(
            let_expression.get_var_name(),
            mapped_var_expr,
            mapped_body_expr,
        )
        .into()
    } else {
        QueryExpression::Let(let_expression.clone())
    }
}

/// Java `QueryExpressionMapper.identity()`.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:96 QueryExpressionMapper.identity
pub fn identity() -> IdentityMapper {
    IdentityMapper
}

/// Returns a `QueryExpressionMapper` which applies all the mappings provided by `mappers`, in
/// the reverse order of mapper array.
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:104 QueryExpressionMapper.compose
pub fn compose<C>(
    mappers: Vec<Box<dyn QueryExpressionVisitor<QueryExpression, C>>>,
) -> ComposedQueryExpressionMapper<C> {
    ComposedQueryExpressionMapper { mappers }
}

// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:109 QueryExpressionMapper.ComposedQueryExpressionMapper
pub struct ComposedQueryExpressionMapper<C> {
    mappers: Vec<Box<dyn QueryExpressionVisitor<QueryExpression, C>>>,
}

impl<C> ComposedQueryExpressionMapper<C> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:141 ComposedQueryExpressionMapper.mapAll
    fn map_all(&mut self, expression: QueryExpression, context: &C) -> QueryExpression {
        let mut expr = expression;
        for mapper in self.mappers.iter_mut().rev() {
            expr = expr.accept(mapper.as_mut(), context);
        }
        expr
    }
}

impl<C> QueryExpressionMapper<C> for ComposedQueryExpressionMapper<C> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:117 ComposedQueryExpressionMapper.visit
    fn map_target_literal(
        &mut self,
        target_literal: &Arc<TargetLiteral>,
        context: &C,
    ) -> QueryExpression {
        self.map_all(
            QueryExpression::TargetLiteral(target_literal.clone()),
            context,
        )
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:122 ComposedQueryExpressionMapper.visit
    fn map_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_all(
            QueryExpression::BinaryOperator(binary_operator_expression.clone()),
            context,
        )
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:127 ComposedQueryExpressionMapper.visit
    fn map_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_all(
            QueryExpression::Function(function_expression.clone()),
            context,
        )
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:132 ComposedQueryExpressionMapper.visit
    fn map_let_expression(
        &mut self,
        let_expression: &Arc<LetExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_all(QueryExpression::Let(let_expression.clone()), context)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:137 ComposedQueryExpressionMapper.visit
    fn map_set_expression(
        &mut self,
        set_expression: &Arc<SetExpression>,
        context: &C,
    ) -> QueryExpression {
        self.map_all(QueryExpression::Set(set_expression.clone()), context)
    }
}

// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:152 QueryExpressionMapper.IdentityMapper
pub struct IdentityMapper;

// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:159 IdentityMapper.visit
// Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:179 IdentityMapper.visit
// (the trait's defaults return the target literal and the set expression unchanged)
impl QueryExpressionMapper<()> for IdentityMapper {
    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:164 IdentityMapper.visit
    fn map_binary_operator_expression(
        &mut self,
        binary_operator_expression: &Arc<BinaryOperatorExpression>,
        _context: &(),
    ) -> QueryExpression {
        QueryExpression::BinaryOperator(binary_operator_expression.clone())
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:169 IdentityMapper.visit
    fn map_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        _context: &(),
    ) -> QueryExpression {
        QueryExpression::Function(function_expression.clone())
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryExpressionMapper.java:174 IdentityMapper.visit
    fn map_let_expression(
        &mut self,
        let_expression: &Arc<LetExpression>,
        _context: &(),
    ) -> QueryExpression {
        QueryExpression::Let(let_expression.clone())
    }
}
