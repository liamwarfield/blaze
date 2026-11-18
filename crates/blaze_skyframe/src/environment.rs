//! Port of `SkyFunction.Environment` (`SkyFunctionEnvironment`) and `SkyframeLookupResult`.

use crate::evaluator::{DoneEntry, DoneKind, Run};
use crate::{ErrorInfo, GroupedDeps, Interrupted, Key, SkyError, SkyFunction, SkyKey, SkyValue};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable, Reportable};
use blaze_util::check_state;
use parking_lot::Mutex;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The result of `getValueOrThrow`: the value (`None` if unavailable), or the dependency's
/// exception of the requested type.
pub type ValueOrThrow<V, E> = Result<Option<V>, E>;

/// What the environment learned about the node's dependencies.
pub(crate) struct EnvState {
    /// Direct deps in request order. Java: `newlyRequestedDepsValues` plus group sizes.
    pub(crate) deps: GroupedDeps,
    /// The entry each dep had when requested.
    pub(crate) requested: HashMap<SkyKey, Arc<DoneEntry>>,
    /// Java: `childErrorInfos` (a `LinkedHashSet`).
    pub(crate) child_error_infos: Vec<Arc<ErrorInfo>>,
    /// Java: `depErrorKey` — the first dep in error, without keep_going.
    pub(crate) dep_error: Option<Arc<ErrorInfo>>,
    pub(crate) aborted_dep: bool,
    pub(crate) in_cycle_dep: bool,
    pub(crate) interrupted_dep: bool,
}

/// What a function sees while computing.
// Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:72 SkyFunctionEnvironment
pub struct Environment<'a> {
    run: &'a Run<'a>,
    key: &'a SkyKey,
    st: EnvState,
    values_missing: Arc<AtomicBool>,
    listener: EnvListener<'a>,
}

impl<'a> Environment<'a> {
    pub(crate) fn new(run: &'a Run<'a>, key: &'a SkyKey, function: &'a dyn SkyFunction) -> Self {
        Environment {
            run,
            key,
            st: EnvState {
                deps: GroupedDeps::new(),
                requested: HashMap::new(),
                child_error_infos: vec![],
                dep_error: None,
                aborted_dep: false,
                in_cycle_dep: false,
                interrupted_dep: false,
            },
            values_missing: Arc::new(AtomicBool::new(false)),
            listener: EnvListener {
                run,
                key,
                function,
                events_to_report: Mutex::new(vec![]),
            },
        }
    }

    pub(crate) fn into_parts(self) -> (EnvState, bool, EnvListener<'a>) {
        let missing = self.values_missing.load(Ordering::SeqCst);
        (self.st, missing, self.listener)
    }

    /// The key being computed.
    pub fn key(&self) -> &SkyKey {
        self.key
    }

    /// The client context set with [`crate::EvaluatorBuilder::client`].
    pub fn client<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.run.ev.client().downcast_ref::<T>()
    }

    /// Returns a reporter for events. Warnings are deduplicated across the evaluator; events that
    /// should be stored for replay (errors, posts with `store_for_replay`) are reported, tagged
    /// with [`SkyFunction::extract_tag`], when the node completes.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:805 SkyFunctionEnvironment.getListener
    pub fn listener(&self) -> &dyn ExtendedEventHandler {
        &self.listener
    }

    /// Java: Environment.valuesMissing
    // Java: com/google/devtools/build/skyframe/AbstractSkyFunctionEnvironment.java:101 AbstractSkyFunctionEnvironment.valuesMissing
    pub fn values_missing(&self) -> bool {
        self.values_missing.load(Ordering::SeqCst)
    }

    /// Always false.
    ///
    /// PORT: Java re-runs the parents of a failed node "in error bubbling" so they can wrap the
    /// error when there is no keep_going. In the blocking engine a parent already observes its
    /// child's error during its normal compute (`get_value_or_throw` returns it in both modes),
    /// which is equivalent, so no evaluation is ever in error bubbling.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:1036 SkyFunctionEnvironment.inErrorBubbling
    pub fn in_error_bubbling(&self) -> bool {
        false
    }

    /// Returns `Err(Interrupted)` if the evaluation was interrupted or is aborting (Java
    /// interrupts in-flight work when a no-keep-going evaluation fails). Long computations that
    /// do not request dependencies should poll this (Java: `Thread.interrupted()`).
    pub fn check_interrupted(&self) -> Result<(), Interrupted> {
        if self.run.ev.interrupt_flag().load(Ordering::SeqCst)
            || self.run.aborting.load(Ordering::SeqCst)
        {
            return Err(Interrupted);
        }
        Ok(())
    }

    fn check_external_interrupt(&self) -> Result<(), Interrupted> {
        if self.run.ev.interrupt_flag().load(Ordering::SeqCst) {
            return Err(Interrupted);
        }
        Ok(())
    }

    /// The deps requested so far, in request order.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:811 SkyFunctionEnvironment.getTemporaryDirectDeps
    pub fn get_temporary_direct_deps(&self) -> &GroupedDeps {
        &self.st.deps
    }

    /// Java: Environment.getValue. `Ok(None)` means the dependency is unavailable (error, cycle,
    /// abort); the function should then return `Ok(None)`.
    pub fn get_value<K: Key>(&mut self, key: K) -> Result<Option<Arc<K::Value>>, Interrupted> {
        Ok(self
            .get_value_dyn(&SkyKey::new(key))?
            .map(downcast_value::<K>))
    }

    /// Untyped `getValue`.
    pub fn get_value_dyn(&mut self, key: &SkyKey) -> Result<Option<SkyValue>, Interrupted> {
        let entry = self.request(key)?;
        Ok(unwrap_or_throw::<()>(
            &entry,
            self.run.ctx.keep_going,
            &self.values_missing,
            |_| None,
        )
        .unwrap_or(None))
    }

    /// Java: Environment.getValueOrThrow(key, E.class). A dependency error whose exception is an
    /// `E` is returned as `Ok(Err(e))` and does not count as missing; other errors give
    /// `Ok(Ok(None))`.
    pub fn get_value_or_throw<K: Key, E>(
        &mut self,
        key: K,
    ) -> Result<ValueOrThrow<Arc<K::Value>, E>, Interrupted>
    where
        E: std::error::Error + Clone + 'static,
    {
        Ok(self
            .get_value_or_throw_dyn::<E>(&SkyKey::new(key))?
            .map(|v| v.map(downcast_value::<K>)))
    }

    /// Untyped `getValueOrThrow`.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:536 SkyFunctionEnvironment.getValueOrThrowInternal
    pub fn get_value_or_throw_dyn<E>(
        &mut self,
        key: &SkyKey,
    ) -> Result<ValueOrThrow<SkyValue, E>, Interrupted>
    where
        E: std::error::Error + Clone + 'static,
    {
        let entry = self.request(key)?;
        Ok(unwrap_or_throw(
            &entry,
            self.run.ctx.keep_going,
            &self.values_missing,
            |e| e.downcast_ref::<E>().cloned(),
        ))
    }

    /// Java: Environment.getValueOrThrow(key, E1.class, E2.class, ...). `catch` inspects the
    /// dependency's exception and returns `Some(t)` for the exception classes the caller asked for
    /// (downcast to several types); only when it returns `None` is the dependency missing.
    pub fn get_value_or_throw_with<K: Key, T>(
        &mut self,
        key: K,
        catch: impl FnOnce(&SkyError) -> Option<T>,
    ) -> Result<Result<Option<Arc<K::Value>>, T>, Interrupted> {
        let entry = self.request(&SkyKey::new(key))?;
        Ok(
            unwrap_or_throw(&entry, self.run.ctx.keep_going, &self.values_missing, catch)
                .map(|v| v.map(downcast_value::<K>)),
        )
    }

    /// Requests a batch of deps, computing unstarted ones in parallel. Read the results with
    /// [`SkyframeLookupResult::get`] / [`SkyframeLookupResult::get_or_throw`].
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:560 SkyFunctionEnvironment.getValuesAndExceptions
    pub fn get_values_and_exceptions(
        &mut self,
        dep_keys: &[SkyKey],
    ) -> Result<SkyframeLookupResult, Interrupted> {
        self.check_external_interrupt()?;
        let mut in_request: HashSet<SkyKey> = HashSet::new();
        let mut missing_keys: Vec<SkyKey> = vec![];
        for k in dep_keys {
            if !in_request.insert(k.clone()) {
                continue; // Duplicate key in this request.
            }
            if !self.st.requested.contains_key(k) {
                missing_keys.push(k.clone());
            }
        }
        let entries = self.run.eval_keys(&missing_keys);
        for (k, e) in missing_keys.iter().zip(entries) {
            self.st.requested.insert(k.clone(), e);
        }
        self.st.deps.append_group(missing_keys);
        let mut result = HashMap::new();
        let mut processed: HashSet<&SkyKey> = HashSet::new();
        for k in dep_keys {
            if !processed.insert(k) {
                continue;
            }
            let entry = self.st.requested[k].clone();
            self.process_dep_value(&entry);
            result.insert(k.clone(), entry);
        }
        Ok(SkyframeLookupResult {
            entries: result,
            keep_going: self.run.ctx.keep_going,
            values_missing: self.values_missing.clone(),
        })
    }

    /// `getValuesAndExceptions` followed by `get` on each key, in the order of `keys`.
    pub fn get_values(&mut self, keys: &[SkyKey]) -> Result<Vec<Option<SkyValue>>, Interrupted> {
        let result = self.get_values_and_exceptions(keys)?;
        Ok(keys.iter().map(|k| result.get(k)).collect())
    }

    /// Typed form of [`Environment::get_values`].
    pub fn get_values_typed<K: Key + Clone>(
        &mut self,
        keys: &[K],
    ) -> Result<Vec<Option<Arc<K::Value>>>, Interrupted> {
        let dyn_keys: Vec<SkyKey> = keys.iter().cloned().map(SkyKey::new).collect();
        Ok(self
            .get_values(&dyn_keys)?
            .into_iter()
            .map(|v| v.map(downcast_value::<K>))
            .collect())
    }

    /// Java: `lookupRequestedDep` + `processDepEntry`/`processDepValue` for a single key.
    fn request(&mut self, key: &SkyKey) -> Result<Arc<DoneEntry>, Interrupted> {
        self.check_external_interrupt()?;
        let entry = match self.st.requested.get(key) {
            Some(e) => e.clone(),
            None => {
                let e = self.run.eval_key(key);
                self.st.requested.insert(key.clone(), e.clone());
                self.st.deps.append_singleton(key.clone());
                e
            }
        };
        self.process_dep_value(&entry);
        Ok(entry)
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:613 SkyFunctionEnvironment.processDepValue
    fn process_dep_value(&mut self, entry: &DoneEntry) {
        if entry.is_undone_view() {
            self.values_missing.store(true, Ordering::SeqCst);
            match entry.kind {
                DoneKind::Aborted => self.st.aborted_dep = true,
                DoneKind::InCycle => self.st.in_cycle_dep = true,
                DoneKind::Interrupted => self.st.interrupted_dep = true,
                DoneKind::Committed | DoneKind::Bubbled => {}
            }
            return;
        }
        let Some(error_info) = &entry.error else {
            return;
        };
        if !self
            .st
            .child_error_infos
            .iter()
            .any(|e| Arc::ptr_eq(e, error_info))
        {
            self.st.child_error_infos.push(error_info.clone());
        }

        // If we get here, then the dep node is present and also (i) depends on a cycle or (ii)
        // errorful or (iii) both. The remaining question is whether or not to convey to the
        // SkyFunction that the dep node is missing.
        //
        // If the dep node depends on a cycle, then we always want the SkyFunction to act as
        // though the dep is missing (cycles are not supposed to be observable by SkyFunctions),
        // so we always set valuesMissing.
        //
        // PORT: Java also sets valuesMissing for an errorful dep without keep_going outside error
        // bubbling. Here a function always observes dep errors as Java does in error bubbling
        // (see `in_error_bubbling`), so only the dep error is recorded.
        if !error_info.get_cycle_info().is_empty() {
            self.values_missing.store(true, Ordering::SeqCst);
        }
        // We arbitrarily record the first child error if we are about to abort.
        if !self.run.ctx.keep_going && self.st.dep_error.is_none() {
            self.st.dep_error = Some(error_info.clone());
        }
    }
}

/// Java: `SkyFunctionEnvironment.unwrapOrThrow` + `handleError`. `catch` returns the typed
/// exception when the dep's exception is one the caller asked for.
fn unwrap_or_throw<T>(
    entry: &DoneEntry,
    keep_going: bool,
    values_missing: &AtomicBool,
    catch: impl FnOnce(&SkyError) -> Option<T>,
) -> Result<Option<SkyValue>, T> {
    if entry.is_undone_view() {
        values_missing.store(true, Ordering::SeqCst);
        return Ok(None);
    }
    let Some(error_info) = &entry.error else {
        return Ok(entry.value.clone());
    };
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:767 SkyFunctionEnvironment.handleError
    if keep_going {
        // In keepGoing mode, returns any computed value to the caller.
        if let Some(v) = &entry.value {
            return Ok(Some(v.clone()));
        }
    }
    match error_info.get_exception() {
        None => {
            // If there's no exception, there must be a cycle.
            check_state!(
                !error_info.get_cycle_info().is_empty(),
                "{:?} {:?}",
                error_info,
                entry
            );
        }
        Some(exception) => {
            // The exception may only propagate in keepGoing mode or during error bubbling (here:
            // always, see `Environment::in_error_bubbling`).
            if let Some(e) = catch(exception) {
                return Err(e);
            }
        }
    }
    values_missing.store(true, Ordering::SeqCst);
    Ok(None)
}

fn downcast_value<K: Key>(v: SkyValue) -> Arc<K::Value> {
    v.downcast::<K::Value>()
        .unwrap_or_else(|_| panic!("value of {:?} has the wrong type", K::FUNCTION))
}

/// The result of [`Environment::get_values_and_exceptions`].
// Java: com/google/devtools/build/skyframe/SkyframeLookupResult.java:30 SkyframeLookupResult
pub struct SkyframeLookupResult {
    entries: HashMap<SkyKey, Arc<DoneEntry>>,
    keep_going: bool,
    values_missing: Arc<AtomicBool>,
}

impl SkyframeLookupResult {
    /// Returns the value for `key`, or `None` if it is unavailable (which marks values missing).
    // Java: com/google/devtools/build/skyframe/SkyframeLookupResult.java:39 SkyframeLookupResult.get
    pub fn get(&self, key: &SkyKey) -> Option<SkyValue> {
        self.get_or_throw_internal::<()>(key, |_| None)
            .unwrap_or(None)
    }

    /// Typed [`SkyframeLookupResult::get`].
    pub fn get_typed<K: Key + Clone>(&self, key: &K) -> Option<Arc<K::Value>> {
        self.get(&SkyKey::new(key.clone())).map(downcast_value::<K>)
    }

    /// Returns the value for `key`, or `Err(e)` when its exception is an `E`.
    // Java: com/google/devtools/build/skyframe/SkyframeLookupResult.java:59 SkyframeLookupResult.getOrThrow
    pub fn get_or_throw<E>(&self, key: &SkyKey) -> Result<Option<SkyValue>, E>
    where
        E: std::error::Error + Clone + 'static,
    {
        self.get_or_throw_internal(key, |e| e.downcast_ref::<E>().cloned())
    }

    /// Typed [`SkyframeLookupResult::get_or_throw`].
    pub fn get_or_throw_typed<K: Key + Clone, E>(&self, key: &K) -> Result<Option<Arc<K::Value>>, E>
    where
        E: std::error::Error + Clone + 'static,
    {
        Ok(self
            .get_or_throw::<E>(&SkyKey::new(key.clone()))?
            .map(downcast_value::<K>))
    }

    /// Java: SkyframeLookupResult.getOrThrow(key, E1.class, E2.class, ...) with a caller-supplied
    /// `catch` (see [`Environment::get_value_or_throw_with`]).
    pub fn get_or_throw_with<K: Key + Clone, T>(
        &self,
        key: &K,
        catch: impl FnOnce(&SkyError) -> Option<T>,
    ) -> Result<Option<Arc<K::Value>>, T> {
        Ok(self
            .get_or_throw_internal(&SkyKey::new(key.clone()), catch)?
            .map(downcast_value::<K>))
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:667 SkyFunctionEnvironment.getOrThrow
    fn get_or_throw_internal<T>(
        &self,
        key: &SkyKey,
        catch: impl FnOnce(&SkyError) -> Option<T>,
    ) -> Result<Option<SkyValue>, T> {
        let Some(entry) = self.entries.get(key) else {
            // Java: BugReport.sendNonFatalBugReport("Value for %s was missing, this should never
            // happen") and returns null.
            return Ok(None);
        };
        unwrap_or_throw(entry, self.keep_going, &self.values_missing, catch)
    }
}

enum StoredReportable {
    Event(Event),
    Post(Arc<dyn Postable>),
}

impl StoredReportable {
    fn store_for_replay(&self) -> bool {
        match self {
            StoredReportable::Event(e) => e.store_for_replay(),
            StoredReportable::Post(p) => p.store_for_replay(),
        }
    }
}

/// The environment as an event handler (Java: `SkyFunctionEnvironment implements
/// ExtendedEventHandler`).
pub(crate) struct EnvListener<'a> {
    run: &'a Run<'a>,
    key: &'a SkyKey,
    function: &'a dyn SkyFunction,
    /// Java: `eventsToReport`.
    events_to_report: Mutex<Vec<StoredReportable>>,
}

impl EnvListener<'_> {
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:1020 SkyFunctionEnvironment.getTagFromKey
    fn get_tag_from_key(&self) -> Option<String> {
        self.function.extract_tag(self.key)
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:829 SkyFunctionEnvironment.reportEvent
    fn report_event(&self, event: StoredReportable) {
        if event.store_for_replay() {
            self.events_to_report.lock().push(event);
        } else {
            let reporter = &*self.run.ctx.event_handler;
            match event {
                StoredReportable::Event(e) => e.report_to(reporter),
                StoredReportable::Post(p) => p.report_to(reporter),
            }
        }
    }

    /// Reports the events stored while the node computed, tagged. Called when the node commits
    /// (or its error bubbles). Events of a computation that ends with missing deps are dropped,
    /// as in Java, where only the run that completes reports them.
    ///
    /// PORT: Java also stores the events on the node to replay them on later builds; one build
    /// per process needs no replay.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:300 SkyFunctionEnvironment.reportEventsAndGetEventsToStore
    pub(crate) fn flush(&self) {
        let events = std::mem::take(&mut *self.events_to_report.lock());
        if events.is_empty() {
            return;
        }
        let tag = self.get_tag_from_key();
        let reporter = &*self.run.ctx.event_handler;
        for event in events {
            match event {
                StoredReportable::Event(e) => e.with_tag(tag.as_deref()).report_to(reporter),
                // Postables have no tag ("No tag-based filtering").
                StoredReportable::Post(p) => p.report_to(reporter),
            }
        }
    }
}

impl EventHandler for EnvListener<'_> {
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:816 SkyFunctionEnvironment.handle
    fn handle(&self, event: &Event) {
        let mut event = event.clone();
        if event.get_kind() == EventKind::Warning {
            event = event.with_tag(self.get_tag_from_key().as_deref());
            if !self.run.ev.emitted_event_state.add_warning(&event) {
                return; // Duplicate warning.
            }
        }
        self.report_event(StoredReportable::Event(event));
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for EnvListener<'_> {
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:825 SkyFunctionEnvironment.post
    fn post(&self, obj: Arc<dyn Postable>) {
        self.report_event(StoredReportable::Post(obj));
    }
}
