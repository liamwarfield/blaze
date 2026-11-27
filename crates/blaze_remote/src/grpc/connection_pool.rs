//! Port of `lib/remote/grpc/{TokenBucket, SharedConnectionFactory, DynamicConnectionPool,
//! ConnectionFactory, ConnectionPool, Connection}.java`: the connections of a
//! `ReferenceCountedChannel`.
//!
//! PORT: Java's `Single`s become futures. A connection is created when the first future that
//! needs it is polled (Java: on subscription); a waiter that is dropped gives up its place
//! (Java: `dispose`). The connection type is generic (Java's `Connection` interface); the channel
//! uses a tonic `Channel`, one HTTP/2 connection each.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::{BoxFuture, FutureExt, Shared};
use parking_lot::Mutex;
use tokio::sync::oneshot;

/// The `IllegalStateException("closed")` of a closed bucket, factory or pool.
fn closed() -> io::Error {
    io::Error::other("IllegalStateException: closed")
}

/// A token bucket used to control the concurrency.
// Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:33 TokenBucket
pub struct TokenBucket<T> {
    state: Mutex<BucketState<T>>,
}

struct BucketState<T> {
    tokens: VecDeque<T>,
    /// The observers waiting for a token, in subscription order (Java: the observers of the
    /// `BehaviorSubject`, which `onNext` visits in order; the first to poll gets the token).
    waiters: VecDeque<oneshot::Sender<T>>,
    closed: bool,
}

impl<T: Send + 'static> Default for TokenBucket<T> {
    fn default() -> Self {
        TokenBucket::new(Vec::new())
    }
}

impl<T: Send + 'static> TokenBucket<T> {
    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:41 TokenBucket.<init>
    pub fn new(initial_tokens: Vec<T>) -> TokenBucket<T> {
        TokenBucket {
            state: Mutex::new(BucketState {
                tokens: initial_tokens.into(),
                waiters: VecDeque::new(),
                closed: false,
            }),
        }
    }

    /// Add a token to the bucket.
    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:51 TokenBucket.addToken
    pub fn add_token(&self, token: T) {
        let mut state = self.state.lock();
        if state.closed {
            // `tokenBehaviorSubject.onNext` after `onComplete` is ignored; the token stays.
            state.tokens.push_back(token);
            return;
        }
        state.tokens.push_back(token);
        // onNext: each observer in order takes the first token, until there is none.
        while !state.tokens.is_empty() {
            let Some(waiter) = state.waiters.pop_front() else {
                break;
            };
            let token = state.tokens.pop_front().expect("checked above");
            if let Err(token) = waiter.send(token) {
                // A disposed observer does not take the token.
                state.tokens.push_front(token);
            }
        }
    }

    /// Returns the number of tokens in the bucket.
    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:57 TokenBucket.size
    pub fn size(&self) -> usize {
        self.state.lock().tokens.len()
    }

    /// Attempts to acquire a token from the bucket.
    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:63 TokenBucket.tryAcquireToken
    pub fn try_acquire_token(&self) -> Option<T> {
        self.state.lock().tokens.pop_front()
    }

    /// Returns a future which will complete when there is a token available in the bucket.
    /// It fails with `IllegalStateException("closed")` once the bucket is closed.
    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:72 TokenBucket.acquireToken
    pub fn acquire_token(
        &self,
    ) -> impl std::future::Future<Output = io::Result<T>> + Send + use<T> {
        let receiver = {
            let mut state = self.state.lock();
            if state.closed {
                Err(closed())
            } else if let Some(token) = state.tokens.pop_front() {
                // The subject replays its last value on subscription.
                Ok(Ok(token))
            } else {
                let (sender, receiver) = oneshot::channel();
                state.waiters.push_back(sender);
                Ok(Err(receiver))
            }
        };
        async move {
            match receiver? {
                Ok(token) => Ok(token),
                // A dropped sender: the bucket was closed (`onComplete`).
                Err(receiver) => receiver.await.map_err(|_| closed()),
            }
        }
    }

    // Java: com/google/devtools/build/lib/remote/grpc/TokenBucket.java:110 TokenBucket.close
    pub fn close(&self) {
        let mut state = self.state.lock();
        state.tokens.clear();
        state.closed = true;
        // onComplete: the waiting observers fail with "closed".
        state.waiters.clear();
    }
}

/// A factory of connections (`ConnectionFactory`).
// Java: com/google/devtools/build/lib/remote/grpc/ConnectionFactory.java:26 ConnectionFactory
pub trait ConnectionFactory<C>: Send + Sync {
    /// Creates a new connection.
    fn create(&self) -> BoxFuture<'static, io::Result<C>>;
}

/// A connection (`Connection.close`).
// Java: com/google/devtools/build/lib/remote/grpc/Connection.java:27 Connection
pub trait Connection: Clone + Send + Sync + 'static {
    fn close(&self);
}

type CreationFuture<C> = Shared<BoxFuture<'static, Result<C, Arc<io::Error>>>>;

/// A [`ConnectionFactory`] which creates one connection that can be shared among callers, at
/// most `max_concurrency` at the same time.
// Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:42 SharedConnectionFactory
pub struct SharedConnectionFactory<C: Connection> {
    token_bucket: Arc<TokenBucket<i32>>,
    factory: Arc<dyn ConnectionFactory<C>>,
    /// Java: `connectionAsyncSubject`: the creation of the underlying connection.
    connection: Arc<Mutex<Option<CreationFuture<C>>>>,
    /// Java: `connectionCreationDisposable`: cancels a creation in flight.
    creation_abort: Arc<Mutex<Option<futures::future::AbortHandle>>>,
}

impl<C: Connection> SharedConnectionFactory<C> {
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:53 SharedConnectionFactory.<init>
    pub fn new(factory: Arc<dyn ConnectionFactory<C>>, max_concurrency: i32) -> Self {
        SharedConnectionFactory {
            token_bucket: Arc::new(TokenBucket::new((0..max_concurrency).collect())),
            factory,
            connection: Arc::new(Mutex::new(None)),
            creation_abort: Arc::new(Mutex::new(None)),
        }
    }

    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:72 SharedConnectionFactory.close
    pub fn close(&self) {
        self.token_bucket.close();
        if let Some(abort) = self.creation_abort.lock().take() {
            // A creation in flight is cancelled; its waiters fail with "closed"
            // (`connectionAsyncSubject.onError(new IllegalStateException("closed"))`).
            abort.abort();
        }
        if let Some(creation) = self.connection.lock().take()
            && let Some(Ok(connection)) = creation.peek()
        {
            connection.close();
        }
    }

    /// `createUnderlyingConnectionIfNot()`: the shared creation, started again after a failure.
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:95 SharedConnectionFactory.createUnderlyingConnectionIfNot
    fn create_underlying_connection_if_not(&self) -> CreationFuture<C> {
        let mut connection = self.connection.lock();
        if let Some(creation) = &*connection
            && !matches!(creation.peek(), Some(Err(_)))
        {
            return creation.clone();
        }
        let (abort, registration) = futures::future::AbortHandle::new_pair();
        *self.creation_abort.lock() = Some(abort);
        let created =
            futures::future::Abortable::new(self.factory.create(), registration).map(|r| match r {
                Ok(result) => result.map_err(Arc::new),
                Err(futures::future::Aborted) => Err(Arc::new(closed())),
            });
        let creation = created.boxed().shared();
        *connection = Some(creation.clone());
        creation
    }

    /// A connection; waits for a token when `max_concurrency` callers hold one.
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:112 SharedConnectionFactory.create
    pub fn create(&self) -> BoxFuture<'static, io::Result<SharedConnection<C>>> {
        let token = self.token_bucket.acquire_token();
        let this = self.clone_handle();
        async move {
            let token = token.await?;
            this.create_with_token(token).await
        }
        .boxed()
    }

    /// A connection if a token is available now, else `None`.
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:117 SharedConnectionFactory.tryCreate
    pub fn try_create(&self) -> Option<BoxFuture<'static, io::Result<SharedConnection<C>>>> {
        let token = self.token_bucket.try_acquire_token()?;
        let this = self.clone_handle();
        Some(async move { this.create_with_token(token).await }.boxed())
    }

    fn clone_handle(&self) -> SharedConnectionFactory<C> {
        SharedConnectionFactory {
            token_bucket: self.token_bucket.clone(),
            factory: self.factory.clone(),
            connection: self.connection.clone(),
            creation_abort: self.creation_abort.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:125 SharedConnectionFactory.createWithToken
    async fn create_with_token(&self, token: i32) -> io::Result<SharedConnection<C>> {
        // doOnError / doOnDispose: the token goes back.
        let mut guard = TokenGuard {
            bucket: self.token_bucket.clone(),
            token: Some(token),
        };
        let creation = self.create_underlying_connection_if_not();
        match creation.await {
            Ok(connection) => Ok(SharedConnection {
                connection,
                token: Arc::new(Mutex::new(guard.token.take().map(|token| TokenGuard {
                    bucket: self.token_bucket.clone(),
                    token: Some(token),
                }))),
            }),
            Err(e) => Err(crate::common::errors::shared(&e)),
        }
    }

    /// The tokens available.
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:142 SharedConnectionFactory.numAvailableConnections
    pub fn num_available_connections(&self) -> usize {
        self.token_bucket.size()
    }
}

/// Gives the token back when dropped.
struct TokenGuard {
    bucket: Arc<TokenBucket<i32>>,
    token: Option<i32>,
}

impl Drop for TokenGuard {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            self.bucket.add_token(token);
        }
    }
}

/// A [`Connection`] which wraps an underlying connection and is shared between consumers.
/// Closing it (or dropping its last handle) gives its token back.
///
/// PORT: Java's `fatalErrorPredicate` (a Netty `Errors.NativeIoException` of the epoll
/// transport drops the shared connection) is not ported: a tonic channel reconnects by itself
/// after a transport failure.
// Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:147 SharedConnectionFactory.SharedConnection
#[derive(Clone)]
pub struct SharedConnection<C> {
    connection: C,
    token: Arc<Mutex<Option<TokenGuard>>>,
}

impl<C> SharedConnection<C> {
    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:188 SharedConnectionFactory.SharedConnection.close
    pub fn close(&self) {
        self.token.lock().take();
    }

    // Java: com/google/devtools/build/lib/remote/grpc/SharedConnectionFactory.java:196 SharedConnectionFactory.SharedConnection.getUnderlyingConnection
    pub fn get_underlying_connection(&self) -> &C {
        &self.connection
    }
}

// PORT: skipped, the fatal-error reset of a shared connection (see SharedConnection): SharedConnectionFactory.isFatalError SharedConnectionFactory.SharedConnection.call

/// A connection pool which creates connections on demand: a new connection once every existing
/// one has `max_concurrency_per_connection` users, up to `max_connections` (0: no limit).
// Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:33 DynamicConnectionPool
pub struct DynamicConnectionPool<C: Connection> {
    connection_factory: Arc<dyn ConnectionFactory<C>>,
    max_concurrency_per_connection: i32,
    max_connections: i32,
    closed: AtomicBool,
    state: Mutex<PoolState<C>>,
}

struct PoolState<C: Connection> {
    factories: Vec<SharedConnectionFactory<C>>,
    index_ticker: i32,
}

impl<C: Connection> DynamicConnectionPool<C> {
    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:48 DynamicConnectionPool.<init>
    pub fn new(
        connection_factory: Arc<dyn ConnectionFactory<C>>,
        max_concurrency_per_connection: i32,
        max_connections: i32,
    ) -> Self {
        DynamicConnectionPool {
            connection_factory,
            max_concurrency_per_connection,
            max_connections,
            closed: AtomicBool::new(false),
            state: Mutex::new(PoolState {
                factories: Vec::new(),
                index_ticker: 0,
            }),
        }
    }

    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:56 DynamicConnectionPool.isClosed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:61 DynamicConnectionPool.close
    pub fn close(&self) {
        if self
            .closed
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let mut state = self.state.lock();
            for factory in &state.factories {
                factory.close();
            }
            state.factories.clear();
        }
    }

    /// The number of connections created so far (`factories.size()`).
    pub fn num_connections(&self) -> usize {
        self.state.lock().factories.len()
    }

    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:73 DynamicConnectionPool.nextFactory
    fn next_factory(state: &mut PoolState<C>) -> &SharedConnectionFactory<C> {
        let index = (state.index_ticker % state.factories.len() as i32).unsigned_abs() as usize;
        state.index_ticker = state.index_ticker.wrapping_add(1);
        &state.factories[index]
    }

    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:79 DynamicConnectionPool.createConnection
    fn create_connection(&self) -> BoxFuture<'static, io::Result<SharedConnection<C>>> {
        if self.closed.load(Ordering::SeqCst) {
            return async { Err(closed()) }.boxed();
        }
        let mut state = self.state.lock();
        for _ in 0..state.factories.len() {
            if let Some(connection) = Self::next_factory(&mut state).try_create() {
                return connection;
            }
        }
        if self.max_connections <= 0 || (state.factories.len() as i32) < self.max_connections {
            let factory = SharedConnectionFactory::new(
                self.connection_factory.clone(),
                self.max_concurrency_per_connection,
            );
            let connection = factory.try_create();
            state.factories.push(factory);
            match connection {
                Some(connection) => connection,
                // Java: `factory.tryCreate()` is null with no tokens (max concurrency 0).
                None => async {
                    Err(io::Error::other(
                        "NullPointerException: no token for a new connection",
                    ))
                }
                .boxed(),
            }
        } else {
            Self::next_factory(&mut state).create()
        }
    }

    /// A shared connection (`Single.defer(this::createConnection)`).
    // Java: com/google/devtools/build/lib/remote/grpc/DynamicConnectionPool.java:108 DynamicConnectionPool.create
    pub fn create(self: &Arc<Self>) -> BoxFuture<'static, io::Result<SharedConnection<C>>> {
        let this = self.clone();
        async move { this.create_connection().await }.boxed()
    }
}
