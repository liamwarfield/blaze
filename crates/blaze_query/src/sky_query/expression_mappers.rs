//! Ports of `query2/RdepsToAllRdepsQueryExpressionMapper.java` and
//! `query2/FilteredDirectRdepsInUniverseExpressionMapper.java`: rewrites of `rdeps` whose
//! universe is the SkyQuery universe.

use std::sync::Arc;

use blaze_cmdline::{TargetPattern, TargetPatternParser, TargetPatternType};
use blaze_packages::target::Target;
use blaze_query_engine::query_expression_mapper::{
    QueryExpressionMapper, default_visit_function_expression,
};
use blaze_query_engine::query_expression_visitor::QueryExpressionVisitor;
use blaze_query_engine::{
    AllRdepsFunction, Argument, ArgumentType, BinaryOperatorExpression, FunctionExpression,
    KindFunction, LetExpression, QueryExpression, QueryFunction, RdepsFunction, SetExpression,
    TargetLiteral,
};

/// Whether `rdeps(<universe>, ...)` can be evaluated as `allrdeps(...)`.
// Java: com/google/devtools/build/lib/query2/RdepsToAllRdepsQueryExpressionMapper.java:83 RdepsToAllRdepsQueryExpressionMapper.Eligibility
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eligibility {
    NotEligible,
    EligibleWithFiltering,
    EligibleAsIs,
}

// Java: com/google/devtools/build/lib/query2/RdepsToAllRdepsQueryExpressionMapper.java:91 RdepsToAllRdepsQueryExpressionMapper.determineEligibility
pub fn determine_eligibility(
    target_pattern_parser: &TargetPatternParser,
    absolute_universe_scope_pattern: &TargetPattern,
    rdeps_universe_pattern_string: &str,
) -> Eligibility {
    let absolute_rdeps_universe_target_pattern = match target_pattern_parser
        .parse(&target_pattern_parser.absolutize(rdeps_universe_pattern_string))
    {
        Ok(p) => p,
        Err(_) => return Eligibility::NotEligible,
    };
    if absolute_universe_scope_pattern.get_type()
        != absolute_rdeps_universe_target_pattern.get_type()
    {
        return Eligibility::NotEligible;
    }
    match absolute_universe_scope_pattern.get_type() {
        TargetPatternType::PathAsTarget | TargetPatternType::SingleTarget => {
            if absolute_universe_scope_pattern.get_original_pattern()
                == absolute_rdeps_universe_target_pattern.get_original_pattern()
            {
                Eligibility::EligibleAsIs
            } else {
                Eligibility::NotEligible
            }
        }
        TargetPatternType::TargetsInPackage | TargetPatternType::TargetsBelowDirectory => {
            if absolute_universe_scope_pattern.get_directory()
                != absolute_rdeps_universe_target_pattern.get_directory()
            {
                return Eligibility::NotEligible;
            }
            // Note: If we're here, both patterns are either TARGETS_IN_PACKAGE or
            // TARGETS_BELOW_DIRECTORY, and are for the same directory.
            if absolute_universe_scope_pattern.get_rules_only()
                == absolute_rdeps_universe_target_pattern.get_rules_only()
            {
                return Eligibility::EligibleAsIs;
            }
            if absolute_universe_scope_pattern.get_rules_only() {
                // If the actual universe is narrower, then allrdeps would be unsound because it
                // may produce narrower results.
                Eligibility::NotEligible
            } else {
                // If the actual universe is wider, then allrdeps would produce wider results.
                // Therefore, we'd want to filter those results.
                Eligibility::EligibleWithFiltering
            }
        }
    }
}

fn new_function_expression(
    function: Arc<dyn QueryFunction<Target>>,
    args: Vec<Argument>,
) -> QueryExpression {
    QueryExpression::Function(Arc::new(FunctionExpression::new(function, args)))
}

/// Rewrites `rdeps(<universe>, x[, depth])` to `allrdeps(x[, depth])` when the universe is the
/// SkyQuery universe.
// Java: com/google/devtools/build/lib/query2/RdepsToAllRdepsQueryExpressionMapper.java:34 RdepsToAllRdepsQueryExpressionMapper
pub struct RdepsToAllRdepsQueryExpressionMapper {
    target_pattern_parser: TargetPatternParser,
    absolute_universe_scope_pattern: TargetPattern,
}

impl RdepsToAllRdepsQueryExpressionMapper {
    // Java: com/google/devtools/build/lib/query2/RdepsToAllRdepsQueryExpressionMapper.java:38 RdepsToAllRdepsQueryExpressionMapper.<init>
    pub fn new(
        target_pattern_parser: TargetPatternParser,
        absolute_universe_scope_pattern: TargetPattern,
    ) -> Self {
        RdepsToAllRdepsQueryExpressionMapper {
            target_pattern_parser,
            absolute_universe_scope_pattern,
        }
    }
}

impl QueryExpressionMapper<()> for RdepsToAllRdepsQueryExpressionMapper {
    // Java: com/google/devtools/build/lib/query2/RdepsToAllRdepsQueryExpressionMapper.java:45 RdepsToAllRdepsQueryExpressionMapper.visit
    fn map_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &(),
    ) -> QueryExpression {
        if function_expression.get_function().get_name() == "rdeps" {
            let args = function_expression.get_args();
            let rdeps_universe_expression = args[0].get_expression();
            if let QueryExpression::TargetLiteral(literal) = rdeps_universe_expression {
                let eligibility = determine_eligibility(
                    &self.target_pattern_parser,
                    &self.absolute_universe_scope_pattern,
                    literal.get_pattern(),
                );
                match eligibility {
                    Eligibility::EligibleAsIs => {
                        return new_function_expression(
                            Arc::new(AllRdepsFunction::new()),
                            args[1..].to_vec(),
                        );
                    }
                    Eligibility::EligibleWithFiltering => {
                        return new_function_expression(
                            Arc::new(KindFunction::new()),
                            vec![
                                Argument::of_word(" rule$"),
                                Argument::of_expression(new_function_expression(
                                    Arc::new(AllRdepsFunction::new()),
                                    args[1..].to_vec(),
                                )),
                            ],
                        );
                    }
                    Eligibility::NotEligible => {
                        // Do nothing. The return statement at the bottom of the method is what we
                        // want.
                    }
                }
            }
        }
        default_visit_function_expression(self, function_expression, context)
    }
}

/// Rewrites `rdeps(f(<universe>), x, 1)`, where `f` is a chain of filtering functions over the
/// SkyQuery universe, into `f(rdeps(<universe>, x, 1))`.
// Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:67 FilteredDirectRdepsInUniverseExpressionMapper
pub struct FilteredDirectRdepsInUniverseExpressionMapper {
    target_pattern_parser: TargetPatternParser,
    absolute_universe_scope_pattern: TargetPattern,
}

impl FilteredDirectRdepsInUniverseExpressionMapper {
    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:72 FilteredDirectRdepsInUniverseExpressionMapper.<init>
    pub fn new(
        target_pattern_parser: TargetPatternParser,
        absolute_universe_scope_pattern: TargetPattern,
    ) -> Self {
        FilteredDirectRdepsInUniverseExpressionMapper {
            target_pattern_parser,
            absolute_universe_scope_pattern,
        }
    }

    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:105 FilteredDirectRdepsInUniverseExpressionMapper.makeUnfilteredRdepsWithDepthOne
    fn make_unfiltered_rdeps_with_depth_one(
        rdeps_universe_argument: Argument,
        source_argument: Argument,
    ) -> QueryExpression {
        new_function_expression(
            Arc::new(RdepsFunction::new()),
            vec![
                rdeps_universe_argument,
                source_argument,
                Argument::of_integer(1),
            ],
        )
    }

    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:112 FilteredDirectRdepsInUniverseExpressionMapper.wrapExprWithFilter
    fn wrap_expr_with_filter(
        cur_function: QueryExpression,
        filter_expr: &Arc<FunctionExpression>,
    ) -> QueryExpression {
        let filter_expr_args = filter_expr.get_args();
        let filtering_function = filter_expr
            .get_typed_function::<Target>()
            .as_filtering_function()
            .expect("a filtering function");
        let mut rewritten_args = Vec::new();
        let mut cur_function = Some(cur_function);
        for (i, arg) in filter_expr_args.iter().enumerate() {
            if i as i32 == filtering_function.get_expression_to_filter_index() {
                rewritten_args.push(Argument::of_expression(
                    cur_function.take().expect("one filtered argument"),
                ));
            } else {
                rewritten_args.push(arg.clone());
            }
        }
        QueryExpression::Function(Arc::new(filter_expr.with_args(rewritten_args)))
    }
}

impl QueryExpressionMapper<()> for FilteredDirectRdepsInUniverseExpressionMapper {
    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:79 FilteredDirectRdepsInUniverseExpressionMapper.visit
    fn map_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        context: &(),
    ) -> QueryExpression {
        if function_expression.get_function().get_name() == "rdeps" {
            let args = function_expression.get_args();
            // This transformation only applies to the 3-arg form of rdeps, with depth == 1.
            if args.len() == 3 && args[2].get_integer() == 1 {
                let rdeps_universe_argument = &args[0];
                let rdeps_universe_expression = rdeps_universe_argument.get_expression();
                let result = rdeps_universe_expression.accept(
                    &mut ExtractFilteringFunctionsFromUniverseVisitor { mapper: self },
                    &(),
                );
                // If we get back a non-empty result, then there are filtering functions that can
                // be safely factored out.
                if let Some(universe_argument) = result.universe_argument {
                    let mut cur_function = Self::make_unfiltered_rdeps_with_depth_one(
                        universe_argument,
                        args[1].clone(),
                    );
                    for filter_expr in &result.filtering_expressions {
                        cur_function = Self::wrap_expr_with_filter(cur_function, filter_expr);
                    }
                    return cur_function;
                }
            }
        }
        default_visit_function_expression(self, function_expression, context)
    }
}

/// The filtering functions over the universe, innermost first, and the universe argument.
// Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:127 FilteredDirectRdepsInUniverseExpressionMapper.ExtractFilteringFunctionsResult
struct ExtractFilteringFunctionsResult {
    filtering_expressions: Vec<Arc<FunctionExpression>>,
    universe_argument: Option<Argument>,
}

impl ExtractFilteringFunctionsResult {
    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:128 FilteredDirectRdepsInUniverseExpressionMapper.ExtractFilteringFunctionsResult.EMPTY
    fn empty() -> Self {
        ExtractFilteringFunctionsResult {
            filtering_expressions: Vec::new(),
            universe_argument: None,
        }
    }
}

// Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:146 FilteredDirectRdepsInUniverseExpressionMapper.ExtractFilteringFunctionsFromUniverseVisitor
struct ExtractFilteringFunctionsFromUniverseVisitor<'a> {
    mapper: &'a FilteredDirectRdepsInUniverseExpressionMapper,
}

impl QueryExpressionVisitor<ExtractFilteringFunctionsResult, ()>
    for ExtractFilteringFunctionsFromUniverseVisitor<'_>
{
    fn visit_target_literal(
        &mut self,
        _target_literal: &Arc<TargetLiteral>,
        _context: &(),
    ) -> ExtractFilteringFunctionsResult {
        ExtractFilteringFunctionsResult::empty()
    }

    fn visit_binary_operator_expression(
        &mut self,
        _binary_operator_expression: &Arc<BinaryOperatorExpression>,
        _context: &(),
    ) -> ExtractFilteringFunctionsResult {
        ExtractFilteringFunctionsResult::empty()
    }

    // Java: com/google/devtools/build/lib/query2/FilteredDirectRdepsInUniverseExpressionMapper.java:79 FilteredDirectRdepsInUniverseExpressionMapper.ExtractFilteringFunctionsFromUniverseVisitor.visit
    fn visit_function_expression(
        &mut self,
        function_expression: &Arc<FunctionExpression>,
        _context: &(),
    ) -> ExtractFilteringFunctionsResult {
        let typed = function_expression.get_typed_function::<Target>();
        let Some(filtering_function) = typed.as_filtering_function() else {
            return ExtractFilteringFunctionsResult::empty();
        };
        let filtered_argument = &function_expression.get_args()
            [filtering_function.get_expression_to_filter_index() as usize];
        blaze_util::check_argument!(filtered_argument.get_type() == ArgumentType::Expression);
        let filtered_expression = filtered_argument.get_expression();
        match filtered_expression {
            QueryExpression::TargetLiteral(literal) => {
                let eligibility = determine_eligibility(
                    &self.mapper.target_pattern_parser,
                    &self.mapper.absolute_universe_scope_pattern,
                    literal.get_pattern(),
                );
                if eligibility == Eligibility::EligibleAsIs
                    || eligibility == Eligibility::EligibleWithFiltering
                {
                    return ExtractFilteringFunctionsResult {
                        filtering_expressions: vec![function_expression.clone()],
                        universe_argument: Some(filtered_argument.clone()),
                    };
                }
            }
            QueryExpression::Function(_) => {
                let recursive_result = filtered_expression.accept(self, &());
                if let Some(universe_argument) = recursive_result.universe_argument {
                    let mut filtering_expressions = recursive_result.filtering_expressions;
                    filtering_expressions.push(function_expression.clone());
                    return ExtractFilteringFunctionsResult {
                        filtering_expressions,
                        universe_argument: Some(universe_argument),
                    };
                }
            }
            _ => {}
        }
        ExtractFilteringFunctionsResult::empty()
    }

    fn visit_let_expression(
        &mut self,
        _let_expression: &Arc<LetExpression>,
        _context: &(),
    ) -> ExtractFilteringFunctionsResult {
        ExtractFilteringFunctionsResult::empty()
    }

    fn visit_set_expression(
        &mut self,
        _set_expression: &Arc<SetExpression>,
        _context: &(),
    ) -> ExtractFilteringFunctionsResult {
        ExtractFilteringFunctionsResult::empty()
    }
}
