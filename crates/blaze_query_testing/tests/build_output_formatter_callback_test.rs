//! Java's `BuildOutputFormatterCallbackTest`
//! (src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java).
//!
//! Every test of the class runs after `setUpCqueryOptions`, which installs the native `MockRule`
//! `my_rule` (`MockRule.define`) with `helper.useRuleClassProvider`; Blaze's rule class providers
//! have no native test rules, so each test is ignored. The inherited `AbstractQueryTest` and
//! `PostAnalysisQueryTest` tests run in `configured_target_query_semantics_test.rs`.

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java:96 BuildOutputFormatterCallbackTest.selectInAttribute
#[test]
#[ignore = "setUpCqueryOptions defines a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn select_in_attribute() {}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java:139 BuildOutputFormatterCallbackTest.alias
#[test]
#[ignore = "setUpCqueryOptions defines a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn alias() {}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java:175 BuildOutputFormatterCallbackTest.aliasWithSelect
#[test]
#[ignore = "setUpCqueryOptions defines a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn alias_with_select() {}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java:226 BuildOutputFormatterCallbackTest.sourceFile
#[test]
#[ignore = "setUpCqueryOptions defines a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn source_file() {}

// Java: src/test/java/com/google/devtools/build/lib/query2/cquery/BuildOutputFormatterCallbackTest.java:247 BuildOutputFormatterCallbackTest.outputFile
#[test]
#[ignore = "setUpCqueryOptions defines a native MockRule (MockRule.define) through useRuleClassProvider; Blaze rule class providers have no native test rules"]
fn output_file() {}
