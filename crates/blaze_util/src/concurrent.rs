//! The classes of `lib/concurrent` and `lib/supplier` that Blaze replaces with Rust primitives.
//!
//! The Skyframe engine is the blocking evaluator of `blaze_skyframe` (decision D5): each evaluation
//! runs on scoped worker threads (`std::thread::scope`) with a parallelism from the options, and
//! a function blocks on its dependencies instead of restarting. The Java executors, visitors and
//! futures that serve the restart model are not ported as classes (docs/decisions.md, 2026-11-19,
//! ruling 2). Where a visitor's behaviour is observable (error propagation, interruption), it is
//! ported at the use (for example `DiskCacheGarbageCollector::run` checks for an interrupt before
//! each directory and each deletion, as `awaitQuiescence(true)` stops the work). The lines below
//! name, per Java method, what plays its role.

// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.createExecutorService
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.createWithExecutorService
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.create
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.awaitQuiescence
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.awaitQuiescenceWithoutShutdown
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.execute
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.executeWithExecutorService
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.executeWrappedRunnable
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.maybeSaveUnhandledThrowable
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.recordError
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.compareTo
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.setInterrupted
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.decrementRemainingTasks
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.blockNewActions
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.getInterruptionLatchForTestingOnly
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.isInterrupted
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.getTaskCount
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.getExecutorService
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.dependOnFuture
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.markFutureDone
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.mustJobsBeStopped
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.shutdownExecutorService
// PORT: skipped, replaced by the evaluator scoped workers and per-use visits: AbstractQueueVisitor.cancelAllFutures
// PORT: skipped, unused at the pin (no other class refers to it): AbstractShardedVisitor.scheduleVisitationsAndAwaitQuiescence
// PORT: skipped, unused at the pin (no other class refers to it): AbstractShardedVisitor.checkComplete
// PORT: skipped, unused at the pin (no other class refers to it): AbstractShardedVisitor.getMaxBatchSize
// PORT: skipped, unused at the pin (no other class refers to it): AbstractShardedVisitor.shardAndScheduleRemainingItems
// PORT: skipped, unused at the pin (no other class refers to it): AbstractShardedVisitor.visit
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): AddressFreer.run
// PORT: skipped, replaced by blaze_cmdline::interner and by-value equality: BlazeInterners.concurrencyLevel
// PORT: skipped, replaced by blaze_cmdline::interner and by-value equality: BlazeInterners.newWeakInterner
// PORT: skipped, replaced by blaze_cmdline::interner and by-value equality: BlazeInterners.newStrongInterner
// PORT: skipped, replaced by blaze_cmdline::interner and by-value equality: BlazeInterners.intern
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.iterator
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.size
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.put
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.offer
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.take
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.poll
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.remainingCapacity
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.drainTo
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: BlockingStack.peek
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.run
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.runIfNotCancelled
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.awaitCompletion
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.awaitCompletionUninterruptibly
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.cancelAndAwait
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.checkCanAwait
// PORT: skipped, replaced by tokio tasks (blaze_remote): CancellableTask.runCompletion
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.tryAppend
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.take
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.size
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.clear
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.toString
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.getQueueForTesting
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.getQueueOffset
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.decrementSkips
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): ConcurrentFifo.incrementSkips
// PORT: skipped, replaced by the evaluator error handling (blaze_skyframe): ErrorClassifier.classifyException
// PORT: skipped, replaced by the evaluator error handling (blaze_skyframe): ErrorClassifier.classify
// PORT: skipped, unused at the pin (no other class refers to it): ErrorHandler.handle
// PORT: skipped, unused at the pin (no other class refers to it): EvictableSupplier.get
// PORT: skipped, unused at the pin (no other class refers to it): EvictableSupplier.computeValue
// PORT: skipped, unused at the pin (no other class refers to it): EvictableSupplier.evictForTesting
// PORT: skipped, unused at the pin (no other class refers to it): EvictableSupplier.peekCachedValue
// PORT: skipped, replaced by joining std threads: ExecutorUtil.uninterruptibleShutdownNow
// PORT: skipped, replaced by joining std threads: ExecutorUtil.interruptibleShutdown
// PORT: skipped, replaced by joining std threads: ExecutorUtil.uninterruptibleShutdown
// PORT: skipped, replaced by joining std threads: ExecutorUtil.shutdownImpl
// PORT: skipped, replaced by joining std threads: ExecutorUtil.newSlackPool
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.create
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.incrementAndGet
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.decrementAndGet
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.asImmutableMap
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.getCounter
// PORT: skipped, unused at the pin (no other class refers to it): FastHotKeyAtomicLongMap.clear
// PORT: skipped, replaced by the evaluator scoped workers: ForkJoinQuiescingExecutor.withOwnershipOf
// PORT: skipped, replaced by the evaluator scoped workers: ForkJoinQuiescingExecutor.setErrorClassifier
// PORT: skipped, replaced by the evaluator scoped workers: ForkJoinQuiescingExecutor.build
// PORT: skipped, replaced by the evaluator scoped workers: ForkJoinQuiescingExecutor.newBuilder
// PORT: skipped, replaced by the evaluator scoped workers: ForkJoinQuiescingExecutor.executeWrappedRunnable
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.getCount
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.tryAcquireShared
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.tryReleaseShared
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.increaseCount
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.await
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.countDown
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.increment
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.toString
// PORT: skipped, replaced by std::thread::scope joins: IncrementableCountDownLatch.awaitUninterruptibly
// PORT: skipped, replaced by blocking evaluation (decision D5): InterruptiblePromptableSupplier.startGet
// PORT: skipped, replaced by blocking evaluation (decision D5): InterruptibleSupplier.get
// PORT: skipped, replaced by parking_lot locks: KeyedLocker.close
// PORT: skipped, replaced by parking_lot locks: KeyedLocker.writeLock
// PORT: skipped, replaced by blocking evaluation (decision D5): MemoizingInterruptibleSupplier.isInitialized
// PORT: skipped, replaced by blocking evaluation (decision D5): MemoizingInterruptibleSupplier.of
// PORT: skipped, replaced by blocking evaluation (decision D5): MemoizingInterruptibleSupplier.get
// PORT: skipped, replaced by synchronous NestedSet expansion (blaze_collect): MoreFutures.waitForAllInterruptiblyFailFast
// PORT: skipped, replaced by synchronous NestedSet expansion (blaze_collect): MoreFutures.allAsListOrCancelAll
// PORT: skipped, replaced by synchronous NestedSet expansion (blaze_collect): MoreFutures.waitForFutureAndGet
// PORT: skipped, replaced by synchronous NestedSet expansion (blaze_collect): MoreFutures.waitForFutureAndGetWithCheckedException
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.createWithExecutorServices
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.execute
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.getExecutorServiceByThreadPoolType
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.shutdownExecutorService
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.internalShutdownExecutorService
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.launchQueuedUpExecutionPhaseTasks
// PORT: skipped, replaced by the evaluator scoped workers: MultiExecutorQueueVisitor.hasSeparatePoolForExecutionTasks
// PORT: skipped, replaced by the evaluator scoped workers: MultiThreadPoolsQuiescingExecutor.execute
// PORT: skipped, replaced by the evaluator scoped workers: MultiThreadPoolsQuiescingExecutor.launchQueuedUpExecutionPhaseTasks
// PORT: skipped, replaced by the evaluator scoped workers: MultiThreadPoolsQuiescingExecutor.hasSeparatePoolForExecutionTasks
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.acquireAll
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.releaseAll
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.estimateCurrentNumUniqueValues
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.unbounded
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.maxNumUniqueValues
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.build
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.newBuilder
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.instance
// PORT: skipped, used by SkyQuery (not ported) and the replaced engine: MultisetSemaphore.computeNumNeededPermitsLocked
// PORT: skipped, replaced by the evaluator scoped workers: NamedForkJoinPool.newNamedPool
// PORT: skipped, replaced by the evaluator scoped workers: NamedForkJoinPool.newThread
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): PaddedAddresses.createPaddedBaseAddress
// PORT: skipped, replaced by the evaluator work queue (blaze_skyframe): PaddedAddresses.getAlignedAddress
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.weakIntern
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.removeWeak
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.intern
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.getPool
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.size
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.shrinkAll
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.shrink
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.shrinkAsNewWeakInterner
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.getMapReflectively
// PORT: skipped, replaced by blaze_cmdline::interner (nothing to shrink): PooledInterner.getOrWeakIntern
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingExecutor.awaitQuiescence
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingExecutor.awaitQuiescenceWithoutShutdown
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingExecutor.dependOnFuture
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingExecutor.getInterruptionLatchForTestingOnly
// PORT: skipped, replaced by EvaluationContext.parallelism (blaze_skyframe): QuiescingExecutors.analysisParallelism
// PORT: skipped, replaced by EvaluationContext.parallelism (blaze_skyframe): QuiescingExecutors.executionParallelism
// PORT: skipped, replaced by EvaluationContext.parallelism (blaze_skyframe): QuiescingExecutors.globbingParallelism
// PORT: skipped, replaced by EvaluationContext.parallelism (blaze_skyframe): QuiescingExecutors.getAnalysisExecutor
// PORT: skipped, replaced by EvaluationContext.parallelism (blaze_skyframe): QuiescingExecutors.getMergedAnalysisAndExecutionExecutor
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.increment
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.decrement
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.notifyException
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.getValue
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.doneWithError
// PORT: skipped, replaced by the evaluator scoped workers: QuiescingFuture.run
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.execute
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.acceptResponse
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.acceptFailure
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.acceptFutureResponse
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.createBatchExecutionStrategy
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.createCallbackBatchExecutionStrategy
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.createPerResponseBatchExecutionStrategy
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.executeBatch
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.create
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.submit
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.toString
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.populateBatch
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.continueToNextBatchOrBecomeIdle
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.request
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.setResponse
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.errorIfFutureUnset
// PORT: skipped, unused at the pin (no other class refers to it): RequestBatcher.setException
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.key
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.tryTakeOwnership
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.completeWith
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.failWith
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.verifyComplete
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.onSuccess
// PORT: skipped, replaced by blocking evaluation (decision D5): SettableFutureKeyedValue.onFailure
// PORT: skipped, replaced by the evaluator sharded maps (blaze_skyframe): Sharder.add
// PORT: skipped, replaced by the evaluator sharded maps (blaze_skyframe): Sharder.immutableListOfLists
// PORT: skipped, replaced by the evaluator sharded maps (blaze_skyframe): Sharder.iterator
// PORT: skipped, unused at the pin (no other class refers to it): StripedKeyedLocker.lockAndMakeAutoUnlocker
// PORT: skipped, unused at the pin (no other class refers to it): StripedKeyedLocker.writeLock
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.execute
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.ongoingExecutionForTesting
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.attributes
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.cancel
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.pendingToString
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.retain
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.activeUses
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.wrap
// PORT: skipped, replaced by tokio tasks (blaze_remote): TaskDeduplicator.run
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.ensureOwner
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.ensureNotJoined
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.ensureJoinedIfOwner
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.defaultThreadFactory
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.open
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.fork
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.join
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.isCancelled
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.onComplete
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.interruptAll
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.cancel
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.close
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.getThreads
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.state
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.get
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.exception
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.run
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.toString
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.allSuccessful
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.anySuccessful
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.allCompleted
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.result
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.allSuccessfulOrThrow
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.anySuccessfulOrThrow
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.voidOrThrow
// PORT: skipped, unused at the pin (no other class refers to it): TaskGroup.getError
// PORT: skipped, replaced by blaze_server::interruptible: Uninterruptibles.callUninterruptibly
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.add
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.poll
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.steal
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.run
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.addTask
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.pollRemainingTasks
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.registerWorker
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.assignWorkerToNewThread
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.deregisterWorker
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.scan
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.pollOrStealOrWaitTask
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.signalOneWorker
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.signalAllWorkers
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.shutdown
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.shutdownNow
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.isShutdown
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.isTerminated
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.awaitTermination
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.getRandomWorker
// PORT: skipped, replaced by std::thread::scope workers (blaze_skyframe): WorkStealingThreadPoolExecutor.execute
