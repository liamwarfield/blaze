//! Port of `query2/query/LabelVisitor.java`.
//!
//! Visit the transitive closure of a label. Primarily used to "fault in" packages to the
//! packageProvider and ensure the necessary targets exists, in advance of the configuration step,
//! which is intolerant of missing packages/targets.
//!
//! LabelVisitor loads packages concurrently where possible, to increase I/O parallelism. However,
//! the public interface is not thread-safe: calls to public methods should not be made
//! concurrently.
//!
//! LabelVisitor is stateful: It remembers the previous visitation and can check its validity on
//! subsequent calls to sync() instead of doing the normal visitation.
//!
//! PORT: Java runs the cross-package visits on a `QuiescingExecutor` with `parallelThreads`
//! threads. Blaze visits on the calling thread: the work that Java submits to the executor is
//! queued and run in FIFO order once the initial targets are visited (Java's single-thread
//! order); same-package visits still run inline, as in Java. Under Java's parallel executor the
//! order of observer calls is not deterministic; the observers query uses only collect sets and
//! report errors.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
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

use crate::pkgcache::{GetTargetError, TargetEdgeObserver, TargetProvider};
use crate::query::aspects;
use crate::query::target_edge_error_observer::TargetEdgeErrorObserver;

/// Attributes of a visitation which determine whether it is up-to-date or not.
// Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:76 LabelVisitor.VisitationAttributes
struct VisitationAttributes {
    /// The labels of the targets to visit (Java compares the target sets, whose equality is
    /// by label).
    targets_to_visit: HashSet<Label>,
    max_depth: Option<i32>,
    success: bool,
}

impl VisitationAttributes {
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:72 LabelVisitor.NONE
    fn none() -> Self {
        VisitationAttributes {
            targets_to_visit: HashSet::new(),
            max_depth: Some(0),
            success: false,
        }
    }

    /// Returns true if and only if this visitation attribute is still up-to-date.
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:87 VisitationAttributes.current
    fn current(&self, last_visitation: &VisitationAttributes) -> bool {
        self.targets_to_visit == last_visitation.targets_to_visit
            && (last_visitation.max_depth.is_none()
                || !should_visit(self.max_depth, last_visitation.max_depth.unwrap()))
    }
}

/// Java's `RECURSION_LIMIT`: the number of same-package visits run inline before handing the
/// next one to the executor.
// Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:152 LabelVisitor.RECURSION_LIMIT
const RECURSION_LIMIT: i32 = 100;

// Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:71 LabelVisitor
pub struct LabelVisitor {
    target_provider: Arc<dyn TargetProvider>,
    edge_filter: DependencyFilter,
    visited_targets: RefCell<HashMap<Label, i32>>,
    last_visitation: RefCell<VisitationAttributes>,
}

impl LabelVisitor {
    /// Construct a LabelVisitor.
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:160 LabelVisitor.<init>
    pub fn new(target_provider: Arc<dyn TargetProvider>, edge_filter: DependencyFilter) -> Self {
        LabelVisitor {
            target_provider,
            edge_filter,
            visited_targets: RefCell::new(HashMap::new()),
            last_visitation: RefCell::new(VisitationAttributes::none()),
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:166 LabelVisitor.syncWithVisitor
    pub fn sync_with_visitor(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        targets_to_visit: &[Target],
        keep_going: bool,
        parallel_threads: i32,
        max_depth: Option<i32>,
        observer: &dyn TargetEdgeObserver,
    ) -> Result<(), InterruptedException> {
        let next_visitation = VisitationAttributes {
            targets_to_visit: targets_to_visit
                .iter()
                .map(|t| t.get_label().clone())
                .collect(),
            max_depth,
            success: false,
        };
        let redo = {
            let last = self.last_visitation.borrow();
            !last.success || !next_visitation.current(&last)
        };
        if redo {
            *self.last_visitation.borrow_mut() = next_visitation;
            let success = self.redo_visitation(
                event_handler,
                targets_to_visit,
                keep_going,
                parallel_threads,
                max_depth,
                observer,
            )?;
            self.last_visitation.borrow_mut().success = success;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:187 LabelVisitor.syncUncached
    pub fn sync_uncached(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        targets_to_visit: &[Target],
        keep_going: bool,
        parallel_threads: i32,
        max_depth: Option<i32>,
        observer: &dyn TargetEdgeObserver,
    ) -> Result<(), InterruptedException> {
        *self.last_visitation.borrow_mut() = VisitationAttributes::none();
        self.redo_visitation(
            event_handler,
            targets_to_visit,
            keep_going,
            parallel_threads,
            max_depth,
            observer,
        )?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:200 LabelVisitor.redoVisitation
    fn redo_visitation(
        &self,
        event_handler: &Arc<dyn ExtendedEventHandler>,
        targets_to_visit: &[Target],
        _keep_going: bool,
        _parallel_threads: i32,
        max_depth: Option<i32>,
        observer: &dyn TargetEdgeObserver,
    ) -> Result<bool, InterruptedException> {
        self.visited_targets.borrow_mut().clear();
        let visitor = Visitor {
            label_visitor: self,
            event_handler,
            max_depth,
            observer,
            error_observer: TargetEdgeErrorObserver::new(),
            queue: RefCell::new(VecDeque::new()),
            interrupted: RefCell::new(None),
        };
        visitor.visit_targets(targets_to_visit);
        // PORT: Java's keepGoing chooses the executor's exception-handling mode for unexpected
        // (crash) exceptions; there are none to handle here.
        visitor.finish()
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:227 LabelVisitor.hasVisited
    pub fn has_visited(&self, target: &Label) -> bool {
        self.visited_targets.borrow().contains_key(target)
    }
}

/// A unit of work Java submits to the executor (`newVisitRunnable`).
struct QueuedVisit {
    from: Option<Target>,
    attr: Option<Attribute>,
    label: Label,
    depth: i32,
    count: i32,
}

// Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:231 LabelVisitor.Visitor
struct Visitor<'a> {
    label_visitor: &'a LabelVisitor,
    event_handler: &'a Arc<dyn ExtendedEventHandler>,
    max_depth: Option<i32>,
    // Observers are called synchronously in Java, may be called concurrently.
    observer: &'a dyn TargetEdgeObserver,
    error_observer: TargetEdgeErrorObserver,
    queue: RefCell<VecDeque<QueuedVisit>>,
    /// Java sets the worker thread's interrupt flag; `finish` then throws.
    interrupted: RefCell<Option<InterruptedException>>,
}

impl Visitor<'_> {
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:274 Visitor.visitTargets
    fn visit_targets(&self, targets: &[Target]) {
        for target in targets {
            self.visit(None, None, target.clone(), 0, 0);
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:281 Visitor.finish
    fn finish(&self) -> Result<bool, InterruptedException> {
        // executor.awaitQuiescence(/*interruptWorkers=*/ true)
        loop {
            let next = self.queue.borrow_mut().pop_front();
            match next {
                Some(v) => self.run_visit(v.from, v.attr, v.label, v.depth, v.count),
                None => break,
            }
        }
        if let Some(e) = self.interrupted.borrow_mut().take() {
            return Err(e);
        }
        Ok(!self.error_observer.has_errors())
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:290 Visitor.enqueueTarget
    fn enqueue_target(
        &self,
        from: Option<&Target>,
        attr: Option<&Attribute>,
        label: &Label,
        depth: i32,
        count: i32,
    ) {
        // Don't perform the targetProvider lookup if at the maximum depth already.
        if let Some(max_depth) = self.max_depth
            && depth >= max_depth
        {
            return;
        }

        // Avoid thread-related overhead when not crossing packages.
        // Can start a new thread when count reaches 100, to prevent infinite recursion.
        if let Some(from) = from
            && from.get_label().get_package_fragment() == label.get_package_fragment()
            && count < RECURSION_LIMIT
        {
            self.run_visit(
                Some(from.clone()),
                attr.cloned(),
                label.clone(),
                depth,
                count + 1,
            );
        } else {
            self.queue.borrow_mut().push_back(QueuedVisit {
                from: from.cloned(),
                attr: attr.cloned(),
                label: label.clone(),
                depth,
                count: 0,
            });
        }
    }

    /// The body of Java's `newVisitRunnable`.
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:307 Visitor.newVisitRunnable
    fn run_visit(
        &self,
        from: Option<Target>,
        attr: Option<Attribute>,
        label: Label,
        depth: i32,
        count: i32,
    ) {
        match self
            .label_visitor
            .target_provider
            .get_target(self.event_handler, &label)
        {
            Ok(target) => self.visit(from.as_ref(), attr.as_ref(), target, depth + 1, count),
            Err(GetTargetError::NoSuchThing(e)) => {
                // @param from may be null
                self.observe_error(from.as_ref(), &label, &e);
            }
            Err(GetTargetError::Interrupted(e)) => {
                // Thread.currentThread().interrupt();
                let mut interrupted = self.interrupted.borrow_mut();
                if interrupted.is_none() {
                    *interrupted = Some(e);
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:330 Visitor.visit
    fn visit(
        &self,
        from: Option<&Target>,
        attribute: Option<&Attribute>,
        target: Target,
        depth: i32,
        count: i32,
    ) {
        if !should_visit(self.max_depth, depth) {
            return;
        }

        if let Some(from) = from {
            self.observe_edge(from, attribute, &target);
            self.visit_aspects_if_required(from, attribute, &target, depth, count);
        }

        self.visit_target_node(&target, depth, count);
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:350 Visitor.visitAspectsIfRequired
    fn visit_aspects_if_required(
        &self,
        from: &Target,
        attribute: Option<&Attribute>,
        to: &Target,
        depth: i32,
        count: i32,
    ) {
        let (Target::Rule(_), Target::Rule(to_rule)) = (from, to) else {
            return;
        };
        let attribute = attribute.expect("NullPointerException: attribute of a rule edge");
        for aspect in aspects::get_aspects(attribute) {
            if aspects::satisfies(
                aspect,
                to_rule.get_rule_class_object().get_advertised_providers(),
            ) {
                for (aspect_attribute, aspect_label) in
                    &aspects::label_deps_from_all_attributes_of_aspect(
                        aspect,
                        &self.label_visitor.edge_filter,
                    )
                {
                    self.enqueue_target(
                        Some(from),
                        Some(aspect_attribute),
                        aspect_label,
                        depth,
                        count,
                    );
                }
            }
        }
    }

    /// Visit the specified target. Called in a worker thread if not recursing.
    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:378 Visitor.visitTargetNode
    fn visit_target_node(&self, target: &Target, depth: i32, count: i32) {
        {
            let mut visited_targets = self.label_visitor.visited_targets.borrow_mut();
            match visited_targets.get(target.get_label()).copied() {
                None => {
                    visited_targets.insert(target.get_label().clone(), depth);
                }
                Some(min_target_depth) => {
                    // If we've already visited this target at a lower depth, we can safely skip
                    // it.
                    if self.max_depth.is_none() || min_target_depth <= depth {
                        return;
                    }
                    // synchronized (visitedTargets)
                    if visited_targets[target.get_label()] <= depth {
                        return;
                    }
                    visited_targets.insert(target.get_label().clone(), depth);
                }
            }
        }

        self.observe_node(target);

        // LabelVisitor has some legacy special handling of OutputFiles.
        if let Target::OutputFile(output_file) = target {
            let rule = Target::Rule(output_file.get_generating_rule().clone());
            self.observe_edge(target, None, &rule);
            self.visit(None, None, rule, depth + 1, count + 1);
        }

        label_visitation_utils::visit_target(
            target,
            &self.label_visitor.edge_filter,
            &mut |_from_target: &Target, attribute: Option<&Attribute>, to_label: &Label| {
                self.enqueue_target(Some(target), attribute, to_label, depth, count);
            },
        );
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:414 Visitor.observeEdge
    fn observe_edge(&self, from: &Target, attribute: Option<&Attribute>, to: &Target) {
        self.observer.edge(from, attribute, to);
        self.error_observer.edge(from, attribute, to);
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:419 Visitor.observeNode
    fn observe_node(&self, target: &Target) {
        self.observer.node(target);
        self.error_observer.node(target);
    }

    // Java: com/google/devtools/build/lib/query2/query/LabelVisitor.java:424 Visitor.observeError
    fn observe_error(&self, from: Option<&Target>, label: &Label, e: &NoSuchThingException) {
        self.observer.missing_edge(from, label, e);
        self.error_observer.missing_edge(from, label, e);
    }
}
