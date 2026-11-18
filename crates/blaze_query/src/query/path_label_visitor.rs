//! Port of `query2/query/PathLabelVisitor.java`: visits a target graph and computes path-related
//! functions (`somepath`, `allpaths`, `rdeps`, `same_pkg_direct_rdeps`) for the graphless query
//! environment.
//!
//! PORT: Java keys its sets and maps by `Target` identity (`HashSet<Target>`, `HashMap<Target,
//! ...>`), so the iteration order of `allPaths`, `rdeps` and `samePkgDirectRdeps` results is
//! not deterministic in Java. Blaze keys them by label in insertion order. Query output sorts
//! these results unless `--order_output=no`.

use std::collections::VecDeque;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::Attribute;
use blaze_packages::attribute::dependency_filter::DependencyFilter;
use blaze_packages::exceptions::NoSuchThingException;
use blaze_packages::target::Target;
use blaze_packages::target::label_visitation_utils;
use blaze_query_engine::InterruptedException;
use blaze_query_engine::query_environment::should_visit;
use indexmap::{IndexMap, IndexSet};

use crate::pkgcache::{GetSiblingsError, GetTargetError, TargetEdgeObserver, TargetProvider};
use crate::query::aspects;

/// What a visit can throw: `NoSuchThingException` (reported as a missing edge) or
/// `InterruptedException`.
enum VisitError {
    NoSuchThing(NoSuchThingException),
    Interrupted(InterruptedException),
}

impl From<GetTargetError> for VisitError {
    fn from(e: GetTargetError) -> Self {
        match e {
            GetTargetError::NoSuchThing(e) => VisitError::NoSuchThing(e),
            GetTargetError::Interrupted(e) => VisitError::Interrupted(e),
        }
    }
}

// Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:50 PathLabelVisitor
pub struct PathLabelVisitor<'a> {
    target_provider: Arc<dyn TargetProvider>,
    edge_filter: DependencyFilter,
    error_observer: &'a dyn TargetEdgeObserver,
}

impl<'a> PathLabelVisitor<'a> {
    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:55 PathLabelVisitor.<init>
    pub fn new(
        target_provider: Arc<dyn TargetProvider>,
        edge_filter: DependencyFilter,
        error_observer: &'a dyn TargetEdgeObserver,
    ) -> Self {
        PathLabelVisitor {
            target_provider,
            edge_filter,
            error_observer,
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:64 PathLabelVisitor.somePath
    pub fn some_path(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        from: &[Target],
        to: &[Target],
    ) -> Result<Vec<Target>, InterruptedException> {
        let mut visitor = Visitor::new(self, event_handler, VisitorMode::SomePath);
        visitor.visit_targets(from)?;
        for t in to {
            if visitor.has_visited(t) {
                let mut result: VecDeque<Target> = VecDeque::new();
                let mut at = t.clone();
                loop {
                    result.push_front(at.clone());
                    match visitor.get_parents(&at) {
                        None => break,
                        Some(pred) if pred.is_empty() => break,
                        Some(pred) => at = pred[0].clone(),
                    }
                }
                return Ok(result.into_iter().collect());
            }
        }
        Ok(vec![])
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:87 PathLabelVisitor.allPaths
    pub fn all_paths(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        from: &[Target],
        to: &[Target],
    ) -> Result<Vec<Target>, InterruptedException> {
        let mut visitor = Visitor::new(self, event_handler, VisitorMode::AllPaths);
        visitor.visit_targets(from)?;
        let mut result: IndexMap<Label, Target> = IndexMap::new();
        let mut work_queue: VecDeque<Target> = VecDeque::new();
        for t in to {
            if visitor.has_visited(t) {
                work_queue.push_back(t.clone());
            }
        }
        while let Some(at) = work_queue.pop_front() {
            if !result.contains_key(at.get_label()) {
                result.insert(at.get_label().clone(), at.clone());
                if let Some(pred) = visitor.get_parents(&at) {
                    work_queue.extend(pred.iter().cloned());
                }
            }
        }
        Ok(result.into_values().collect())
    }

    /// PORT: Java visits `t.getPackage().getTargets().values()`; targets do not point to their
    /// package here, so the package's targets come from
    /// `TargetProvider.getSiblingTargetsInPackage` (the same targets, in package order).
    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:112 PathLabelVisitor.samePkgDirectRdeps
    pub fn same_pkg_direct_rdeps(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        from: &[Target],
    ) -> Result<Vec<Target>, InterruptedException> {
        let mut visitor = Visitor::new(self, event_handler, VisitorMode::SamePkgDirectRdeps);
        for t in from {
            let package_targets = match self
                .target_provider
                .get_sibling_targets_in_package(event_handler, t)
            {
                Ok(targets) => targets,
                Err(GetSiblingsError::Interrupted(e)) => return Err(e),
                Err(GetSiblingsError::NoSuchPackage(e)) => {
                    panic!("IllegalStateException: package of a loaded target not found: {e}")
                }
            };
            visitor.visit_targets(&package_targets)?;
        }
        let mut result: IndexMap<Label, Target> = IndexMap::new();
        for t in from {
            if let Some(pred) = visitor.get_parents(t) {
                for p in pred {
                    result.insert(p.get_label().clone(), p.clone());
                }
            }
        }
        Ok(result.into_values().collect())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:126 PathLabelVisitor.rdeps
    pub fn rdeps(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        from: &[Target],
        universe: &[Target],
        depth: Option<i32>,
    ) -> Result<Vec<Target>, InterruptedException> {
        let mut visitor = Visitor::new(self, event_handler, VisitorMode::AllPaths);
        visitor.visit_targets(universe)?;

        let mut result: IndexMap<Label, Target> = IndexMap::new();
        let mut at: IndexMap<Label, Target> = IndexMap::new();
        for t in from {
            if visitor.has_visited(t) {
                at.insert(t.get_label().clone(), t.clone());
            }
        }

        let mut next: IndexMap<Label, Target> = IndexMap::new();
        let mut i = 0;
        while should_visit(depth, {
            let d = i;
            i += 1;
            d
        }) && !at.is_empty()
        {
            for t in at.values() {
                if !result.contains_key(t.get_label()) {
                    result.insert(t.get_label().clone(), t.clone());
                    if let Some(pred) = visitor.get_parents(t) {
                        for p in pred {
                            next.insert(p.get_label().clone(), p.clone());
                        }
                    }
                }
            }
            at.clear();
            std::mem::swap(&mut at, &mut next);
        }
        Ok(result.into_values().collect())
    }
}

// Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:157 PathLabelVisitor.VisitorMode
#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitorMode {
    #[allow(dead_code)]
    Deps,
    AllPaths,
    SomePath,
    SamePkgDirectRdeps,
}

// Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:164 PathLabelVisitor.Visit
struct Visit {
    from: Option<Target>,
    attribute: Option<Attribute>,
    target: Target,
}

// Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:184 PathLabelVisitor.Visitor
struct Visitor<'v, 'a> {
    outer: &'v PathLabelVisitor<'a>,
    event_handler: &'v Arc<dyn ExtendedEventHandler>,
    mode: VisitorMode,
    visited: IndexSet<Label>,
    parent_map: IndexMap<Label, Vec<Target>>,
    work_queue: VecDeque<Visit>,
}

impl<'v, 'a> Visitor<'v, 'a> {
    fn new(
        outer: &'v PathLabelVisitor<'a>,
        event_handler: &'v Arc<dyn ExtendedEventHandler>,
        mode: VisitorMode,
    ) -> Self {
        Visitor {
            outer,
            event_handler,
            mode,
            visited: IndexSet::new(),
            parent_map: IndexMap::new(),
            work_queue: VecDeque::new(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:197 Visitor.hasVisited
    fn has_visited(&self, target: &Target) -> bool {
        self.visited.contains(target.get_label())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:202 Visitor.getParents
    fn get_parents(&self, target: &Target) -> Option<&Vec<Target>> {
        self.parent_map.get(target.get_label())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:207 Visitor.visitTargets
    fn visit_targets(&mut self, targets: &[Target]) -> Result<(), InterruptedException> {
        for t in targets {
            self.enqueue_target(None, None, t.clone());
        }
        while let Some(visit) = self.work_queue.pop_front() {
            match self.visit(visit.from.as_ref(), visit.attribute.as_ref(), &visit.target) {
                Ok(()) => {}
                Err(VisitError::NoSuchThing(e)) => {
                    // TODO(ulfjack): We should report the origin of the missing target.
                    self.outer.error_observer.missing_edge(
                        visit.from.as_ref(),
                        visit.target.get_label(),
                        &e,
                    );
                }
                Err(VisitError::Interrupted(e)) => return Err(e),
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:222 Visitor.enqueue
    fn enqueue_label(
        &mut self,
        from: &Target,
        attribute: Option<&Attribute>,
        label: &Label,
    ) -> Result<(), VisitError> {
        if self.mode == VisitorMode::SamePkgDirectRdeps
            && label.get_package_identifier() != from.get_label().get_package_identifier()
        {
            return Ok(());
        }
        let target = self
            .outer
            .target_provider
            .get_target(self.event_handler, label)?;
        self.enqueue_target(Some(from.clone()), attribute.cloned(), target);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:233 Visitor.enqueue
    fn enqueue_target(
        &mut self,
        from: Option<Target>,
        attribute: Option<Attribute>,
        target: Target,
    ) {
        self.work_queue.push_back(Visit {
            from,
            attribute,
            target,
        });
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:237 Visitor.visit
    fn visit(
        &mut self,
        from: Option<&Target>,
        attribute: Option<&Attribute>,
        target: &Target,
    ) -> Result<(), VisitError> {
        if let Some(from) = from {
            match self.mode {
                VisitorMode::Deps => {
                    // Don't update parentMap; only use visited.
                }
                VisitorMode::SamePkgDirectRdeps => {
                    // Only track same-package dependencies.
                    if target.get_label().get_package_identifier()
                        == from.get_label().get_package_identifier()
                    {
                        self.parent_map
                            .entry(target.get_label().clone())
                            .or_default()
                            .push(from.clone());
                    }
                    // We only need to perform a single level of visitation. We have a non-null
                    // 'from' target, and we're now at 'target' target, so we have one level, and
                    // can return here.
                    return Ok(());
                }
                VisitorMode::AllPaths => {
                    self.parent_map
                        .entry(target.get_label().clone())
                        .or_default()
                        .push(from.clone());
                }
                VisitorMode::SomePath => {
                    self.parent_map
                        .entry(target.get_label().clone())
                        .or_insert_with(|| vec![from.clone()]);
                }
            }
            self.visit_aspects_if_required(from, attribute, target)?;
        } else if self.mode == VisitorMode::SomePath {
            self.parent_map
                .entry(target.get_label().clone())
                .or_insert_with(Vec::new);
        }

        if self.visited.insert(target.get_label().clone()) {
            self.visit_edges_of_target(target)?;
        }
        Ok(())
    }

    /// Java stops at the first label whose `enqueue` throws (the exception escapes
    /// `LabelVisitationUtils.visitTarget`); later labels of the target are not enqueued.
    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:278 Visitor.visitEdgesOfTarget
    fn visit_edges_of_target(&mut self, target: &Target) -> Result<(), VisitError> {
        let mut edges: Vec<(Target, Option<Attribute>, Label)> = Vec::new();
        label_visitation_utils::visit_target(
            target,
            &self.outer.edge_filter,
            &mut |from: &Target, attribute: Option<&Attribute>, label: &Label| {
                edges.push((from.clone(), attribute.cloned(), label.clone()));
            },
        );
        for (from, attribute, label) in edges {
            self.enqueue_label(&from, attribute.as_ref(), &label)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/PathLabelVisitor.java:300 Visitor.visitAspectsIfRequired
    fn visit_aspects_if_required(
        &mut self,
        from: &Target,
        attribute: Option<&Attribute>,
        to: &Target,
    ) -> Result<(), VisitError> {
        let (Target::Rule(_), Target::Rule(_)) = (from, to) else {
            return Ok(());
        };
        let attribute = attribute.expect("NullPointerException: attribute of a rule edge");
        for aspect in aspects::get_aspects(attribute) {
            let all_labels =
                aspects::label_deps_from_all_attributes_of_aspect(aspect, &self.outer.edge_filter);
            if all_labels.is_empty() {
                continue;
            }
            if aspects::satisfies_unported(aspect) {
                // PORT: Java collects into a HashMultimap (unordered); the definition order is
                // kept here.
                for (aspect_attribute, aspect_label) in &all_labels {
                    self.enqueue_label(from, Some(aspect_attribute), aspect_label)?;
                }
            }
        }
        Ok(())
    }
}
