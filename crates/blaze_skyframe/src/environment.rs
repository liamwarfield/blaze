//! Port of `SkyFunction.Environment` (`SkyFunctionEnvironment`) and `SkyframeLookupResult`.

use crate::evaluator::{DoneEntry, DoneKind, ERROR_TRANSIENCE_KEY, Run};
use crate::{ErrorInfo, GroupedDeps, Interrupted, Key, SkyError, SkyFunction, SkyKey, SkyValue};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable, Reportable};
use blaze_util::{check_argument, check_state};
use parking_lot::Mutex;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The result of `getValueOrThrow`: the value (`None` if unavailable), or the dependency's
/// exception of the requested type.
pub type ValueOrThrow<V, E> = Result<Option<V>, E>;

/// Java: the `bubbleErrorInfo` map of `ParallelEvaluator.bubbleErrorUp`: the errors built so far
/// for the nodes on the bubbling path. They are never written to the graph.
pub(crate) type BubbleErrorInfo = HashMap<SkyKey, Arc<DoneEntry>>;

/// The entry each dep had when this environment requested it. Java:
/// `previouslyRequestedDepsValues` plus `newlyRequestedDepsValues`. Shared with the
/// [`SkyframeLookupResult`]s of the environment, which in Java are the environment itself.
pub(crate) type RequestedDeps = Arc<Mutex<HashMap<SkyKey, Arc<DoneEntry>>>>;

/// What the environment learned about the node's dependencies.
pub(crate) struct EnvState {
    /// Direct deps in request order. Java: `newlyRequestedDepsValues` plus group sizes.
    pub(crate) deps: GroupedDeps,
    /// The entry each dep had when requested.
    pub(crate) requested: RequestedDeps,
    /// Java: `childErrorInfos` (a `LinkedHashSet`).
    pub(crate) child_error_infos: Vec<Arc<ErrorInfo>>,
    /// Java: `depErrorKey` — the first dep in error, without keep_going (with its error).
    pub(crate) dep_error: Option<(SkyKey, Arc<ErrorInfo>)>,
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
    /// This is not `None` only during error bubbling. When it is not `None`, values in this map
    /// are used over values from the graph for keys present in this map.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:110 bubbleErrorInfo
    bubble_error_info: Option<&'a BubbleErrorInfo>,
    /// Java: `previouslyRequestedDeps` of an environment made by `createForError`: the deps the
    /// node declared during normal evaluation. Empty outside error bubbling.
    previously_requested_deps: GroupedDeps,
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:112 encounteredErrorDuringBubbling
    encountered_error_during_bubbling: bool,
}

impl<'a> Environment<'a> {
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:164 SkyFunctionEnvironment.create
    pub(crate) fn new(run: &'a Run<'a>, key: &'a SkyKey, function: &'a dyn SkyFunction) -> Self {
        Environment {
            run,
            key,
            st: EnvState {
                deps: GroupedDeps::new(),
                requested: Arc::new(Mutex::new(HashMap::new())),
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
            bubble_error_info: None,
            previously_requested_deps: GroupedDeps::new(),
            encountered_error_during_bubbling: false,
        }
    }

    /// The environment a node on the error-bubbling path is re-run with: dependency requests
    /// compute nothing. A dep is the error built for it during bubbling, or what is done in the
    /// graph, or missing.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:192 SkyFunctionEnvironment.createForError
    pub(crate) fn create_for_error(
        run: &'a Run<'a>,
        key: &'a SkyKey,
        function: &'a dyn SkyFunction,
        previously_requested_deps: GroupedDeps,
        bubble_error_info: &'a BubbleErrorInfo,
    ) -> Self {
        Environment {
            bubble_error_info: Some(bubble_error_info),
            previously_requested_deps,
            ..Environment::new(run, key, function)
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

    /// Whether this function is being re-run by `ParallelEvaluator.bubbleErrorUp`: the
    /// evaluation failed (first error without keep_going, or a catastrophe) and the evaluator
    /// walks up from the failed node to a top-level node, giving each node on the way one chance
    /// to throw a better error. Only then (and with keep_going) does a dependency's exception
    /// reach the function through `get_value_or_throw`; requests compute nothing new.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:1036 SkyFunctionEnvironment.inErrorBubbling
    pub fn in_error_bubbling(&self) -> bool {
        self.bubble_error_info.is_some()
    }

    /// Returns `Err(Interrupted)` if the evaluation was interrupted or is aborting (Java
    /// interrupts in-flight work when a no-keep-going evaluation fails). Long computations that
    /// do not request dependencies should poll this (Java: `Thread.interrupted()`).
    ///
    /// During error bubbling it returns `Err(Interrupted)` once a dependency in error was
    /// requested: Java sets the thread's interrupt bit there "to try to prevent the calling
    /// SkyFunction from doing anything fancy after this" (`processDepValue`).
    pub fn check_interrupted(&self) -> Result<(), Interrupted> {
        if self.run.ev.interrupt_flag().load(Ordering::SeqCst) {
            return Err(Interrupted);
        }
        let stop = match self.bubble_error_info {
            Some(_) => self.encountered_error_during_bubbling,
            None => self.run.aborting.load(Ordering::SeqCst),
        };
        if stop {
            return Err(Interrupted);
        }
        Ok(())
    }

    /// Java: `SkyFunctionEnvironment.getExceptionLatchForTesting()` is released (it stays
    /// released during error bubbling).
    #[cfg(test)]
    pub(crate) fn exception_latch_released_for_testing(&self) -> bool {
        self.bubble_error_info.is_some() || self.run.exception_latch_released_for_testing()
    }

    fn check_external_interrupt(&self) -> Result<(), Interrupted> {
        if self.run.ev.interrupt_flag().load(Ordering::SeqCst) {
            return Err(Interrupted);
        }
        Ok(())
    }

    /// The deps requested so far, in request order. During error bubbling: the deps the node
    /// declared during normal evaluation.
    ///
    /// PORT: Java returns the deps of earlier restarts. There are no restarts here, so normal
    /// evaluation returns what this (only) run has requested so far.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:811 SkyFunctionEnvironment.getTemporaryDirectDeps
    pub fn get_temporary_direct_deps(&self) -> &GroupedDeps {
        if self.bubble_error_info.is_some() {
            return &self.previously_requested_deps;
        }
        &self.st.deps
    }

    /// PORT: not ported. Java registers deps without requesting their values (used by
    /// `SkyframeLookupResult`-free producers and `StateMachine` drivers).
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:1041 SkyFunctionEnvironment.registerDependencies
    pub fn register_dependencies(&mut self, _keys: &[SkyKey]) {
        blaze_util::unported!("SkyFunctionEnvironment.registerDependencies")
    }

    /// PORT: not ported. `SkyKeyComputeState` keeps work between restarts of one function; the
    /// blocking engine never restarts a function during normal evaluation.
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:1110 SkyFunctionEnvironment.getState
    pub fn get_state<T>(&mut self, _state_supplier: impl FnOnce() -> T) -> &mut T {
        blaze_util::unported!("SkyFunctionEnvironment.getState")
    }

    /// PORT: not ported. Java lets a function depend on an external `ListenableFuture` and be
    /// restarted when it completes; a blocking function waits for the future itself.
    // Java: com/google/devtools/build/skyframe/SkyFunction.java:433 SkyFunction.Environment.dependOnFuture
    pub fn depend_on_future(&mut self) {
        blaze_util::unported!("SkyFunctionEnvironment.dependOnFuture")
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
            self.in_error_bubbling(),
            &self.values_missing,
            |_| None,
        )
        .unwrap_or(None))
    }

    /// Java: Environment.getValueOrThrow(key, E.class). With keep_going or during error bubbling,
    /// a dependency error whose exception is an `E` is returned as `Ok(Err(e))` and does not
    /// count as missing; other errors give `Ok(Ok(None))`. Without keep_going and outside error
    /// bubbling a dependency in error is always `Ok(Ok(None))` and marks values missing: the
    /// function returns `Ok(None)` and is re-run in error bubbling if it is on the path from the
    /// failed node to a top-level node ([`Environment::in_error_bubbling`]).
    ///
    /// PORT: Java matches subclasses of `E` (`SkyFunctionException.throwIfInstanceOf`); the
    /// downcast here matches the one concrete type `E`. To catch several exception types, or what
    /// is a class hierarchy in Java, use [`Environment::get_value_or_throw_with`].
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
            self.in_error_bubbling(),
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
    ) -> Result<ValueOrThrow<Arc<K::Value>, T>, Interrupted> {
        let entry = self.request(&SkyKey::new(key))?;
        Ok(unwrap_or_throw(
            &entry,
            self.run.ctx.keep_going,
            self.in_error_bubbling(),
            &self.values_missing,
            catch,
        )
        .map(|v| v.map(downcast_value::<K>)))
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
        {
            let requested = self.st.requested.lock();
            for k in dep_keys {
                self.check_not_error_transience(k);
                if !in_request.insert(k.clone()) {
                    continue; // Duplicate key in this request.
                }
                if !requested.contains_key(k) {
                    missing_keys.push(k.clone());
                }
            }
        }
        let entries = self.lookup(&missing_keys);
        {
            let mut requested = self.st.requested.lock();
            for (k, e) in missing_keys.iter().zip(entries) {
                requested.insert(k.clone(), e);
            }
        }
        self.st.deps.append_group(missing_keys);
        let mut processed: HashSet<&SkyKey> = HashSet::new();
        for k in dep_keys {
            if !processed.insert(k) {
                continue;
            }
            let entry = self.st.requested.lock()[k].clone();
            self.process_dep_value(k, &entry);
        }
        // Java returns `this`: the result reads every dep the environment has requested.
        Ok(SkyframeLookupResult {
            requested: self.st.requested.clone(),
            keep_going: self.run.ctx.keep_going,
            in_error_bubbling: self.in_error_bubbling(),
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

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:494 SkyFunctionEnvironment.lookupRequestedDep
    fn check_not_error_transience(&self, dep_key: &SkyKey) {
        check_argument!(
            *dep_key != *ERROR_TRANSIENCE_KEY,
            "Error transience key cannot be in requested deps of {:?}",
            self.key
        );
    }

    /// Fetches deps this environment has not requested before.
    ///
    /// Normal evaluation: computes them (or waits for them), Java's `graph.get` after the restarts
    /// that made the deps done. Error bubbling: nothing is computed. A dep is its entry in
    /// `bubbleErrorInfo`, else its done value in the graph, else missing (`NULL_MARKER`).
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:494 SkyFunctionEnvironment.lookupRequestedDep
    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:601 SkyFunctionEnvironment.processDepEntry
    fn lookup(&self, keys: &[SkyKey]) -> Vec<Arc<DoneEntry>> {
        match self.bubble_error_info {
            Some(bubble_error_info) => keys
                .iter()
                .map(|k| {
                    bubble_error_info
                        .get(k)
                        .cloned()
                        .or_else(|| self.run.ev.committed_entry(k))
                        .unwrap_or_else(|| {
                            Arc::new(DoneEntry::undone(DoneKind::Aborted, GroupedDeps::new()))
                        })
                })
                .collect(),
            None => {
                let entries = self.run.eval_keys(keys, Some(self.key));
                entries
                    .into_iter()
                    .map(|requested| self.run.entry_seen_in_normal_evaluation(requested))
                    .collect()
            }
        }
    }

    /// Java: `lookupRequestedDep` + `processDepEntry`/`processDepValue` for a single key.
    fn request(&mut self, key: &SkyKey) -> Result<Arc<DoneEntry>, Interrupted> {
        self.check_external_interrupt()?;
        self.check_not_error_transience(key);
        let known = self.st.requested.lock().get(key).cloned();
        let entry = match known {
            Some(e) => e,
            None => {
                let e = self
                    .lookup(std::slice::from_ref(key))
                    .pop()
                    .expect("one entry per key");
                self.st.requested.lock().insert(key.clone(), e.clone());
                self.st.deps.append_singleton(key.clone());
                e
            }
        };
        self.process_dep_value(key, &entry);
        Ok(entry)
    }

    // Java: com/google/devtools/build/skyframe/SkyFunctionEnvironment.java:613 SkyFunctionEnvironment.processDepValue
    fn process_dep_value(&mut self, dep_key: &SkyKey, entry: &DoneEntry) {
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
        let keep_going = self.run.ctx.keep_going;
        if self.bubble_error_info.is_some() {
            self.encountered_error_during_bubbling = true;
            // Set interrupted status, to try to prevent the calling SkyFunction from doing
            // anything fancy after this. SkyFunctions executed during error bubbling are supposed
            // to (quickly) rethrow errors or return a value/null (but there's currently no way to
            // enforce this).
            // PORT: the "interrupted status" is `encountered_error_during_bubbling`, which
            // `check_interrupted` reads.
        }

        // If we get here, then the dep node is present and also (i) depends on a cycle or (ii)
        // errorful or (iii) both. The remaining question is whether or not to convey to the
        // SkyFunction that the dep node is missing.
        //
        // If the dep node depends on a cycle, then we always want the SkyFunction to act as
        // though the dep is missing (cycles are not supposed to be observable by SkyFunctions),
        // so we always set valuesMissing.
        //
        // If the dep node is errorful and we're in nokeep_going mode and not in error bubbling,
        // then the SkyFunction is not supposed to be able to observe the error and is supposed to
        // act like the dep is missing, so we set valuesMissing. In contrast, if we are in error
        // bubbling, then the SkyFunction is supposed to be able to observe the error (so as to
        // have the chance to produce an enriched error).
        //
        // If the dep node is errorful and we're in keep_going mode, then SkyFunction is supposed
        // to be able to observe the error (say, with a followup SkyframeLookupResult#getOrThrow)
        // so we don't set valuesMissing.
        if !error_info.get_cycle_info().is_empty()
            || (error_info.get_exception().is_some()
                && !keep_going
                && self.bubble_error_info.is_none())
        {
            self.values_missing.store(true, Ordering::SeqCst);
            // We arbitrarily record the first child error if we are about to abort.
            if !keep_going && self.st.dep_error.is_none() {
                self.st.dep_error = Some((dep_key.clone(), error_info.clone()));
            }
        }
    }
}

/// Java: `SkyFunctionEnvironment.unwrapOrThrow` + `handleError`. `catch` returns the typed
/// exception when the dep's exception is one the caller asked for.
fn unwrap_or_throw<T>(
    entry: &DoneEntry,
    keep_going: bool,
    in_error_bubbling: bool,
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
            // The exception may only propagate in keepGoing mode or during error bubbling.
            if (keep_going || in_error_bubbling)
                && let Some(e) = catch(exception)
            {
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
    /// Every dep the environment has requested (Java: the result is the environment itself, and
    /// `getOrThrow` reads `maybeGetValueFromErrorOrDeps`).
    requested: RequestedDeps,
    keep_going: bool,
    in_error_bubbling: bool,
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

    /// Returns the value for `key`, or `Err(e)` when its exception is an `E` (one concrete type;
    /// see [`Environment::get_value_or_throw`]) and the evaluation is keep_going or in error
    /// bubbling.
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
        // Java: maybeGetValueFromErrorOrDeps(depKey): any dep this environment requested, by
        // this call or by any other `get_value` / `get_values_and_exceptions` call.
        let entry = self.requested.lock().get(key).cloned();
        let Some(entry) = entry else {
            // The key was never requested from this environment. Java:
            // BugReport.sendNonFatalBugReport("Value for %s was missing, this should never
            // happen") and returns null without marking values missing.
            return Ok(None);
        };
        unwrap_or_throw(
            &entry,
            self.keep_going,
            self.in_error_bubbling,
            &self.values_missing,
            catch,
        )
    }

    /// PORT: not ported (callback form of `getOrThrow`).
    // Java: com/google/devtools/build/skyframe/SkyframeLookupResult.java:93 SkyframeLookupResult.queryDep
    pub fn query_dep(&self, _key: &SkyKey) -> bool {
        blaze_util::unported!("SkyframeLookupResult.queryDep")
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

    /// Reports the events stored while the node computed, tagged. Called when the node commits,
    /// and after its error-bubbling run (`bubbleErrorUp` reports the events of each function it
    /// re-runs). Events of a run in normal evaluation that ends with missing deps are dropped, as
    /// in Java, where only the run that completes reports them.
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
