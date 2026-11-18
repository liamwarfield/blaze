//! Ports `packages/EnvironmentGroup.java` and `packages/EnvironmentLabels.java`.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_collect::NestedSetBuilder;
use blaze_collect::Order;
use blaze_events::Event;
use blaze_starlark::Location;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::attribute::AttributeMap;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::exceptions::PackageLoadingCode;
use crate::package::Package;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::target::Target;
use crate::target::TargetData;
use crate::types::AttrValue;
use crate::types::License;
use crate::types::Type;

/// The labels of an environment group (Java `EnvironmentLabels`).
// Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:39 EnvironmentLabels
#[derive(Debug, Clone)]
pub struct EnvironmentLabels {
    pub label: Label,
    pub environments: IndexSet<Label>,
    pub defaults: IndexSet<Label>,
    /// Java `fulfillersMap` (environment to the environments that transitively fulfill it), set
    /// by `processMemberEnvironments`.
    // PORT: java-hash-order. Java uses a HashMap (and a HashMultimap of direct fulfillers); the
    // order is only seen by analysis-time constraint checking, never in loading output, so
    // insertion order is used.
    pub fulfillers_map: Option<IndexMap<Label, NestedSet<Label>>>,
}

impl EnvironmentLabels {
    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:50 EnvironmentLabels.<init>
    pub fn new(label: Label, environments: Vec<Label>, defaults: Vec<Label>) -> Self {
        EnvironmentLabels {
            label,
            environments: environments.into_iter().collect(),
            defaults: defaults.into_iter().collect(),
            fulfillers_map: None,
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:69 EnvironmentLabels.assertNotInitialized
    fn assert_not_initialized(&self) {
        blaze_util::check_state!(self.fulfillers_map.is_none(), "{}", self);
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:73 EnvironmentLabels.checkInitialized
    fn check_initialized(&self) {
        if self.fulfillers_map.is_none() {
            panic!("NullPointerException: {}", self);
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:77 EnvironmentLabels.setFulfillersMap
    fn set_fulfillers_map(&mut self, fulfillers_map: IndexMap<Label, NestedSet<Label>>) {
        blaze_util::check_state!(self.fulfillers_map.is_none(), "{}", self);
        self.fulfillers_map = Some(fulfillers_map);
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:82 EnvironmentLabels.getEnvironments
    pub fn get_environments(&self) -> &IndexSet<Label> {
        self.check_initialized();
        &self.environments
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:87 EnvironmentLabels.getDefaults
    pub fn get_defaults(&self) -> &IndexSet<Label> {
        self.check_initialized();
        &self.defaults
    }

    /// Whether `environment` is a default (false if it does not belong to this group).
    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:96 EnvironmentLabels.isDefault
    pub fn is_default(&self, environment: &Label) -> bool {
        self.check_initialized();
        self.defaults.contains(environment)
    }

    /// The environments that transitively fulfill `environment` (empty if none).
    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:111 EnvironmentLabels.getFulfillers
    pub fn get_fulfillers(&self, environment: &Label) -> NestedSet<Label> {
        self.check_initialized();
        match self.fulfillers_map.as_ref().unwrap().get(environment) {
            Some(ans) => ans.clone(),
            None => NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:117 EnvironmentLabels.getLabel
    pub fn get_label(&self) -> &Label {
        self.check_initialized();
        &self.label
    }
}

impl PartialEq for EnvironmentLabels {
    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:173 EnvironmentLabels.equals
    fn eq(&self, that: &EnvironmentLabels) -> bool {
        self.check_initialized();
        if std::ptr::eq(self, that) {
            return true;
        }
        that.check_initialized();
        self.label == that.label
            && self.environments == that.environments
            && self.defaults == that.defaults
            && fulfiller_maps_equal(
                self.fulfillers_map.as_ref().unwrap(),
                that.fulfillers_map.as_ref().unwrap(),
            )
    }
}

impl Eq for EnvironmentLabels {}

// Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:152 EnvironmentLabels.fulfillerMapsEqual
fn fulfiller_maps_equal(
    map1: &IndexMap<Label, NestedSet<Label>>,
    map2: &IndexMap<Label, NestedSet<Label>>,
) -> bool {
    if std::ptr::eq(map1, map2) {
        return true;
    }
    if map1.len() != map2.len() {
        return false;
    }
    for (key, value) in map1 {
        match map2.get(key) {
            None => return false,
            Some(second_value) => {
                // Do shallowEquals check first for speed.
                if !value.shallow_equals(Some(second_value))
                    && *value.to_list() != *second_value.to_list()
                {
                    return false;
                }
            }
        }
    }
    true
}

impl fmt::Display for EnvironmentLabels {
    // Java: com/google/devtools/build/lib/packages/EnvironmentLabels.java:123 EnvironmentLabels.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fulfillers_size = self.fulfillers_map.as_ref().map_or(0, IndexMap::len);
        let java_set = |s: &IndexSet<Label>| {
            let items: Vec<String> = s.iter().map(|l| l.to_string()).collect();
            format!("[{}]", items.join(", "))
        };
        let fulfillers = match &self.fulfillers_map {
            None => "null".to_owned(),
            Some(m) => {
                let items: Vec<String> = m.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("{{{}}}", items.join(", "))
            }
        };
        // PORT: Java's toString throws a NullPointerException when fulfillersMap is null (it is
        // only called from failing preconditions); the size is printed as 0 instead.
        write!(
            f,
            "EnvironmentLabels{{label={}, sizes={}, {}, {}, environments={}, defaults={}, \
             fulfillersMap={}}}",
            self.label,
            self.environments.len(),
            self.defaults.len(),
            fulfillers_size,
            java_set(&self.environments),
            java_set(&self.defaults),
            fulfillers
        )
    }
}

/// An `environment_group` target (Java `EnvironmentGroup`).
// Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:65 EnvironmentGroup
#[derive(Debug, Clone)]
pub struct EnvironmentGroup {
    environment_labels: EnvironmentLabels,
    location: Location,
    package_metadata: Arc<PackageMetadata>,
    package_declarations: Arc<PackageDeclarations>,
}

impl EnvironmentGroup {
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:86 EnvironmentGroup.<init>
    pub fn new(
        label: Label,
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
        environments: Vec<Label>,
        defaults: Vec<Label>,
        location: Location,
    ) -> Self {
        EnvironmentGroup {
            environment_labels: EnvironmentLabels::new(label, environments, defaults),
            location,
            package_metadata,
            package_declarations,
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:108 EnvironmentGroup.getEnvironmentLabels
    pub fn get_environment_labels(&self) -> &EnvironmentLabels {
        self.environment_labels.check_initialized();
        &self.environment_labels
    }

    /// Checks that environments and defaults are in this package and defaults are members.
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:123 EnvironmentGroup.validateMembership
    pub fn validate_membership(&self) -> Vec<Event> {
        let mut events = Vec::new();

        // All environments should belong to the same package as this group.
        // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:81 EnvironmentGroup.DifferentPackage.apply
        let containing_package_name = self.package_metadata.get_name();
        for environment in self
            .environment_labels
            .environments
            .iter()
            .filter(|e| e.get_package_name() != containing_package_name)
        {
            events.push(Package::error(
                Some(&self.location),
                &format!(
                    "{} is not in the same package as group {}",
                    environment, self.environment_labels.label
                ),
                PackageLoadingCode::EnvironmentInDifferentPackage,
            ));
        }

        // The defaults must be a subset of the member environments.
        for unknown_default in self
            .environment_labels
            .defaults
            .iter()
            .filter(|d| !self.environment_labels.environments.contains(*d))
        {
            events.push(Package::error(
                Some(&self.location),
                &format!(
                    "default {} is not a declared environment for group {}",
                    unknown_default,
                    self.get_label()
                ),
                PackageLoadingCode::DefaultEnvironmentUndeclared,
            ));
        }

        events
    }

    /// Checks member environments exist and computes the fulfillers map.
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:160 EnvironmentGroup.processMemberEnvironments
    pub fn process_member_environments(
        &mut self,
        pkg_targets: &IndexMap<String, Target>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        // Maps an environment to the environments that directly fulfill it.
        // PORT: java-hash-order (HashMultimap); see EnvironmentLabels.fulfillers_map.
        let mut direct_fulfillers: IndexMap<Label, IndexSet<Label>> = IndexMap::new();

        for env_name in &self.environment_labels.environments {
            let env = pkg_targets.get(env_name.get_name());
            if self.is_valid_environment(env, env_name, "", &mut events) {
                let rule = env
                    .unwrap()
                    .as_rule()
                    .expect("ClassCastException: not a Rule");
                let attr = NonconfigurableAttributeMapper::of(&**rule);
                let fulfills = match attr.get("fulfills", Type::LabelList) {
                    Some(AttrValue::LabelList(l)) => l,
                    other => panic!("ClassCastException: expected List<Label>, got {other:?}"),
                };
                for fulfilled_env in &fulfills {
                    if self.is_valid_environment(
                        pkg_targets.get(fulfilled_env.get_name()),
                        fulfilled_env,
                        &format!("in \"fulfills\" attribute of {env_name}: "),
                        &mut events,
                    ) {
                        direct_fulfillers
                            .entry(fulfilled_env.clone())
                            .or_default()
                            .insert(env_name.clone());
                    }
                }
            }
        }

        let mut fulfillers_map: IndexMap<Label, NestedSet<Label>> = IndexMap::new();
        // Now that we know which environments directly fulfill each other, compute which
        // environments transitively fulfill each other. We could alternatively compute this
        // on-demand, but since we don't expect these chains to be very large we opt toward
        // computing them once at package load time.
        self.environment_labels.assert_not_initialized();
        for env_name in &self.environment_labels.environments {
            set_transitive_fulfillers(env_name, &direct_fulfillers, &mut fulfillers_map);
        }

        self.environment_labels.set_fulfillers_map(fulfillers_map);
        events
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:219 EnvironmentGroup.isValidEnvironment
    fn is_valid_environment(
        &self,
        env: Option<&Target>,
        env_name: &Label,
        prefix: &str,
        events: &mut Vec<Event>,
    ) -> bool {
        match env {
            None => {
                events.push(Package::error(
                    Some(&self.location),
                    &format!("{prefix}environment {env_name} does not exist"),
                    PackageLoadingCode::EnvironmentDoesNotExist,
                ));
                false
            }
            Some(env) if env.get_target_kind() != "environment rule" => {
                events.push(Package::error(
                    Some(&self.location),
                    &format!("{prefix}{} is not a valid environment", env.get_label()),
                    PackageLoadingCode::EnvironmentInvalid,
                ));
                false
            }
            Some(env)
                if !self
                    .environment_labels
                    .environments
                    .contains(env.get_label()) =>
            {
                events.push(Package::error(
                    Some(&self.location),
                    &format!("{prefix}{} is not a member of this group", env.get_label()),
                    PackageLoadingCode::EnvironmentNotInGroup,
                ));
                false
            }
            Some(_) => true,
        }
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:248 EnvironmentGroup.getEnvironments
    pub fn get_environments(&self) -> &IndexSet<Label> {
        &self.environment_labels.environments
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:256 EnvironmentGroup.getDefaults
    pub fn get_defaults(&self) -> &IndexSet<Label> {
        &self.environment_labels.defaults
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:261 EnvironmentGroup.getLabel
    pub fn get_label(&self) -> &Label {
        &self.environment_labels.label
    }

    pub fn get_name(&self) -> &str {
        self.environment_labels.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:271 EnvironmentGroup.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:276 EnvironmentGroup.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:291 EnvironmentGroup.getLicense
    pub fn get_license(&self) -> License {
        License::no_license()
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:307 EnvironmentGroup.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        None
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:312 EnvironmentGroup.getVisibility
    pub fn get_visibility(&self) -> RuleVisibility {
        // No rule should be referencing an environment_group.
        // (We override getRawVisibility() separately so as to not display this value during
        // introspection.)
        RuleVisibility::Private
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:320 EnvironmentGroup.isConfigurable
    pub fn is_configurable(&self) -> bool {
        false
    }

    /// Java `EnvironmentGroup.targetKind()`: `"environment group"`.
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:324 EnvironmentGroup.targetKind
    pub fn target_kind() -> &'static str {
        "environment group"
    }
}

impl fmt::Display for EnvironmentGroup {
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:301 EnvironmentGroup.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}",
            EnvironmentGroup::target_kind(),
            self.get_label()
        )
    }
}

impl TargetData for EnvironmentGroup {
    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:281 EnvironmentGroup.getTargetKind
    fn get_target_kind(&self) -> String {
        EnvironmentGroup::target_kind().to_owned()
    }

    // Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:296 EnvironmentGroup.getLocation
    fn get_location(&self) -> Location {
        self.location.clone()
    }

    fn get_label(&self) -> &Label {
        &self.environment_labels.label
    }
}

/// Computes the environments that transitively fulfill `env`, records them in
/// `transitive_fulfillers` and returns them.
// Java: com/google/devtools/build/lib/packages/EnvironmentGroup.java:197 EnvironmentGroup.setTransitiveFulfillers
fn set_transitive_fulfillers(
    env: &Label,
    direct_fulfillers: &IndexMap<Label, IndexSet<Label>>,
    transitive_fulfillers: &mut IndexMap<Label, NestedSet<Label>>,
) -> NestedSet<Label> {
    if let Some(set) = transitive_fulfillers.get(env) {
        set.clone()
    } else if !direct_fulfillers.contains_key(env) {
        // Nobody fulfills this environment.
        let empty_set = NestedSetBuilder::empty_set(Order::STABLE_ORDER);
        transitive_fulfillers.insert(env.clone(), empty_set.clone());
        empty_set
    } else {
        let mut set = NestedSetBuilder::stable_order();
        for fulfilling_env in &direct_fulfillers[env] {
            set.add(fulfilling_env.clone());
            let transitive =
                set_transitive_fulfillers(fulfilling_env, direct_fulfillers, transitive_fulfillers);
            set.add_transitive(&transitive);
        }
        let built_set = set.build();
        transitive_fulfillers.insert(env.clone(), built_set.clone());
        built_set
    }
}
