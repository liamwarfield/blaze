//! Ports of `analysis/ConfigurationMakeVariableContext.java` and
//! `analysis/MakeVariableSupplier.java`.

use std::collections::BTreeMap;

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_packages::package::PackageDeclarations;
use indexmap::IndexMap;

use crate::rule::stringtemplate::{ExpansionException, TemplateContext};

/// Supplies values for make variables.
// Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:26 MakeVariableSupplier
pub trait MakeVariableSupplier: Send + Sync {
    /// Returns Make variable value or `None` if value is not supplied.
    fn get_make_variable(&self, variable_name: &str) -> Result<Option<String>, ExpansionException>;

    /// Returns all Make variables that it supplies.
    fn get_all_make_variables(&self) -> Result<IndexMap<String, String>, ExpansionException>;
}

/// `MakeVariableSupplier` that reads variables from a list of `TemplateVariableInfo` providers
/// (given as their variable maps).
// Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:37 MakeVariableSupplier.TemplateVariableInfoBackedMakeVariableSupplier
pub struct TemplateVariableInfoBackedMakeVariableSupplier {
    template_variable_providers: Vec<IndexMap<String, String>>,
}

impl TemplateVariableInfoBackedMakeVariableSupplier {
    pub fn new(template_variable_providers: Vec<IndexMap<String, String>>) -> Self {
        TemplateVariableInfoBackedMakeVariableSupplier {
            template_variable_providers,
        }
    }
}

impl MakeVariableSupplier for TemplateVariableInfoBackedMakeVariableSupplier {
    // Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:47 TemplateVariableInfoBackedMakeVariableSupplier.getMakeVariable
    fn get_make_variable(&self, variable_name: &str) -> Result<Option<String>, ExpansionException> {
        for template_variable_info in &self.template_variable_providers {
            if let Some(v) = template_variable_info.get(variable_name) {
                return Ok(Some(v.clone()));
            }
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:57 TemplateVariableInfoBackedMakeVariableSupplier.getAllMakeVariables
    fn get_all_make_variables(&self) -> Result<IndexMap<String, String>, ExpansionException> {
        let mut variables: BTreeMap<String, String> = BTreeMap::new();
        for template_variable_info in &self.template_variable_providers {
            for (k, v) in template_variable_info {
                variables.insert(k.clone(), v.clone());
            }
        }
        Ok(variables.into_iter().collect())
    }
}

/// `MakeVariableSupplier` that reads variables from a map.
// Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:67 MakeVariableSupplier.MapBackedMakeVariableSupplier
pub struct MapBackedMakeVariableSupplier {
    make_variables: IndexMap<String, String>,
}

impl MapBackedMakeVariableSupplier {
    pub fn new(make_variables: IndexMap<String, String>) -> Self {
        MapBackedMakeVariableSupplier { make_variables }
    }
}

impl MakeVariableSupplier for MapBackedMakeVariableSupplier {
    // Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:77 MapBackedMakeVariableSupplier.getMakeVariable
    fn get_make_variable(&self, variable_name: &str) -> Result<Option<String>, ExpansionException> {
        Ok(self.make_variables.get(variable_name).cloned())
    }

    // Java: com/google/devtools/build/lib/analysis/MakeVariableSupplier.java:82 MapBackedMakeVariableSupplier.getAllMakeVariables
    fn get_all_make_variables(&self) -> Result<IndexMap<String, String>, ExpansionException> {
        Ok(self.make_variables.clone())
    }
}

/// Implements make variable expansion for make variables that depend on the configuration and
/// the target (not on behavior of the `TransitiveInfoCollection` implementation).
// Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:33 ConfigurationMakeVariableContext
pub struct ConfigurationMakeVariableContext {
    all_make_variable_suppliers: Vec<Box<dyn MakeVariableSupplier>>,
}

impl ConfigurationMakeVariableContext {
    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:37 ConfigurationMakeVariableContext.<init>
    pub fn new(
        pkg_declarations: &PackageDeclarations,
        configuration: &BuildConfigurationValue,
        rule_template_variable_providers: Vec<IndexMap<String, String>>,
    ) -> ConfigurationMakeVariableContext {
        Self::with_extra_suppliers(
            pkg_declarations,
            configuration,
            rule_template_variable_providers,
            Vec::new(),
        )
    }

    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:45 ConfigurationMakeVariableContext.<init>
    pub fn with_extra_suppliers(
        pkg_declarations: &PackageDeclarations,
        configuration: &BuildConfigurationValue,
        rule_template_variable_providers: Vec<IndexMap<String, String>>,
        extra_make_variable_suppliers: Vec<Box<dyn MakeVariableSupplier>>,
    ) -> ConfigurationMakeVariableContext {
        let mut all: Vec<Box<dyn MakeVariableSupplier>> = extra_make_variable_suppliers;
        all.push(Box::new(MapBackedMakeVariableSupplier::new(
            configuration.get_command_line_build_variables().clone(),
        )));
        all.push(Box::new(MapBackedMakeVariableSupplier::new(
            pkg_declarations.get_make_environment(),
        )));
        all.push(Box::new(
            TemplateVariableInfoBackedMakeVariableSupplier::new(rule_template_variable_providers),
        ));
        all.push(Box::new(MapBackedMakeVariableSupplier::new(
            configuration.get_global_make_environment().clone(),
        )));
        ConfigurationMakeVariableContext {
            all_make_variable_suppliers: all,
        }
    }

    /// Returns all make variables, later suppliers' values overridden by earlier ones (Java
    /// `Dict.Builder` over the reversed supplier list: the last put wins at its first position).
    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:74 ConfigurationMakeVariableContext.collectMakeVariables
    pub fn collect_make_variables(&self) -> Result<IndexMap<String, String>, ExpansionException> {
        let mut map: IndexMap<String, String> = IndexMap::new();
        for supplier in self.all_make_variable_suppliers.iter().rev() {
            for (k, v) in supplier.get_all_make_variables()? {
                map.insert(k, v);
            }
        }
        Ok(map)
    }
}

impl TemplateContext for ConfigurationMakeVariableContext {
    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:63 ConfigurationMakeVariableContext.lookupVariable
    fn lookup_variable(&self, name: &str) -> Result<String, ExpansionException> {
        for supplier in &self.all_make_variable_suppliers {
            if let Some(variable_value) = supplier.get_make_variable(name)? {
                return Ok(variable_value);
            }
        }
        Err(ExpansionException(format!("$({name}) not defined")))
    }

    // Java: com/google/devtools/build/lib/analysis/ConfigurationMakeVariableContext.java:83 ConfigurationMakeVariableContext.lookupFunction
    fn lookup_function(&self, name: &str, _param: &str) -> Result<String, ExpansionException> {
        Err(ExpansionException(format!("$({name}) not defined")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_variable_supplier_first_wins_and_all_sorted() {
        let mut a = IndexMap::new();
        a.insert("CC".to_owned(), "gcc".to_owned());
        let mut b = IndexMap::new();
        b.insert("CC".to_owned(), "clang".to_owned());
        b.insert("AR".to_owned(), "ar".to_owned());
        let s = TemplateVariableInfoBackedMakeVariableSupplier::new(vec![a, b]);
        assert_eq!(s.get_make_variable("CC").unwrap(), Some("gcc".to_owned()));
        let all = s.get_all_make_variables().unwrap();
        // Java: TreeMap, later providers overwrite.
        assert_eq!(
            all.into_iter().collect::<Vec<_>>(),
            vec![
                ("AR".to_owned(), "ar".to_owned()),
                ("CC".to_owned(), "clang".to_owned())
            ]
        );
    }
}
