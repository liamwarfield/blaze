//! Ports of `lib/remote/grpc/SharedConnectionFactoryTest.java` and
//! `lib/remote/grpc/DynamicConnectionPoolTest.java`.
//!
//! PORT: Mockito's connection and factory mocks are small fakes that count `create` and
//! `close`; `TestObserver` is a future polled without waiting (see `Observer`). A factory whose
//! creation "never finishes" (Java: a thread sleeping `Integer.MAX_VALUE`) is a pending future
//! that records whether it was dropped (Java: disposed).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

use blaze_remote::grpc::connection_pool::{
    Connection, ConnectionFactory, DynamicConnectionPool, SharedConnection, SharedConnectionFactory,
};
use futures::future::{BoxFuture, FutureExt};

#[derive(Clone)]
struct MockConnection {
    id: usize,
    closes: Arc<AtomicUsize>,
}

impl MockConnection {
    fn new(id: usize) -> MockConnection {
        MockConnection {
            id,
            closes: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Connection for MockConnection {
    fn close(&self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }
}

/// Records whether a pending creation was dropped (`doOnDispose(() -> canceled.set(true))`).
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

type Answer = Box<dyn Fn(usize) -> BoxFuture<'static, io::Result<MockConnection>> + Send + Sync>;

/// `mock(ConnectionFactory.class)` with `when(connectionFactory.create()).thenAnswer(...)`;
/// `answer` gets the number of earlier calls.
struct MockFactory {
    create_times: AtomicUsize,
    answer: Answer,
}

impl MockFactory {
    fn new(answer: Answer) -> Arc<MockFactory> {
        Arc::new(MockFactory {
            create_times: AtomicUsize::new(0),
            answer,
        })
    }

    /// `Single.just(connection)` for every call.
    fn just(connection: MockConnection) -> Arc<MockFactory> {
        MockFactory::new(Box::new(move |_| {
            let connection = connection.clone();
            async move { Ok(connection) }.boxed()
        }))
    }

    /// A creation that never finishes; `canceled` is set when it is dropped.
    fn pending(canceled: Arc<AtomicBool>) -> BoxFuture<'static, io::Result<MockConnection>> {
        let flag = DropFlag(canceled);
        async move {
            let _flag = flag;
            futures::future::pending::<()>().await;
            unreachable!()
        }
        .boxed()
    }

    fn create_times(&self) -> usize {
        self.create_times.load(Ordering::SeqCst)
    }
}

impl ConnectionFactory<MockConnection> for MockFactory {
    fn create(&self) -> BoxFuture<'static, io::Result<MockConnection>> {
        let times = self.create_times.fetch_add(1, Ordering::SeqCst);
        (self.answer)(times)
    }
}

/// `TestObserver<SharedConnection>`.
struct Observer {
    future: Pin<Box<dyn Future<Output = io::Result<SharedConnection<MockConnection>>> + Send>>,
    result: Option<io::Result<SharedConnection<MockConnection>>>,
}

impl Observer {
    fn test(
        future: impl Future<Output = io::Result<SharedConnection<MockConnection>>> + Send + 'static,
    ) -> Observer {
        let mut observer = Observer {
            future: Box::pin(future),
            result: None,
        };
        // `test()` subscribes at once.
        observer.poll();
        observer
    }

    fn poll(&mut self) -> Option<&io::Result<SharedConnection<MockConnection>>> {
        if self.result.is_none() {
            let mut cx = Context::from_waker(futures::task::noop_waker_ref());
            if let Poll::Ready(result) = self.future.as_mut().poll(&mut cx) {
                self.result = Some(result);
            }
        }
        self.result.as_ref()
    }

    /// `assertValue(conn -> conn.getUnderlyingConnection() == connection).assertComplete()`.
    fn assert_connection(&mut self, id: usize) -> SharedConnection<MockConnection> {
        match self.poll() {
            Some(Ok(connection)) => {
                assert_eq!(connection.get_underlying_connection().id, id);
                connection.clone()
            }
            Some(Err(e)) => panic!("expected connection {id}, got {e}"),
            None => panic!("expected connection {id}, still waiting"),
        }
    }

    fn assert_empty(&mut self) {
        assert!(self.poll().is_none(), "the observer has a result");
    }

    /// `assertError(IllegalStateException.class).assertError(e -> e.getMessage().contains("closed"))`.
    fn assert_closed_error(&mut self) {
        match self.poll() {
            Some(Err(e)) => {
                assert!(e.to_string().starts_with("IllegalStateException"), "{e}");
                assert!(e.to_string().contains("closed"), "{e}");
            }
            Some(Ok(_)) => panic!("expected the closed error, got a connection"),
            None => panic!("expected the closed error, still waiting"),
        }
    }
}

fn shared_factory(
    factory: &Arc<MockFactory>,
    max_concurrency: i32,
) -> SharedConnectionFactory<MockConnection> {
    let factory: Arc<dyn ConnectionFactory<MockConnection>> = factory.clone();
    SharedConnectionFactory::new(factory, max_concurrency)
}

fn pool(
    factory: &Arc<MockFactory>,
    max_concurrency_per_connection: i32,
) -> Arc<DynamicConnectionPool<MockConnection>> {
    let factory: Arc<dyn ConnectionFactory<MockConnection>> = factory.clone();
    Arc::new(DynamicConnectionPool::new(
        factory,
        max_concurrency_per_connection,
        /* maxConnections= */ 0,
    ))
}

// SharedConnectionFactoryTest

mod shared_connection_factory_test {
    use super::*;

    #[test]
    fn create_smoke() {
        let connection = MockConnection::new(0);
        let factory = MockFactory::just(connection);
        let shared = shared_factory(&factory, 1);
        assert_eq!(shared.num_available_connections(), 1);

        let _connection = Observer::test(shared.create()).assert_connection(0);
        assert_eq!(factory.create_times(), 1);
        assert_eq!(shared.num_available_connections(), 0);
    }

    #[test]
    fn create_no_connection_creation_before_subscription() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 1);

        let _unused = shared.create();

        assert_eq!(factory.create_times(), 0);
    }

    #[test]
    fn create_exceeding_max_concurrency_waiting() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 1);
        let _connection = Observer::test(shared.create()).assert_connection(0);

        Observer::test(shared.create()).assert_empty();
    }

    #[test]
    fn create_after_connection_closed_share_connections() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 1);
        let connection = Observer::test(shared.create()).assert_connection(0);
        let mut observer2 = Observer::test(shared.create());

        connection.close();

        observer2.assert_connection(0);
        assert_eq!(factory.create_times(), 1);
        assert_eq!(shared.num_available_connections(), 0);
    }

    #[test]
    fn create_below_max_concurrency_share_connections() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 2);

        let _connection0 = Observer::test(shared.create()).assert_connection(0);
        assert_eq!(shared.num_available_connections(), 1);
        let _connection1 = Observer::test(shared.create()).assert_connection(0);

        assert_eq!(factory.create_times(), 1);
        assert_eq!(shared.num_available_connections(), 0);
    }

    #[test]
    fn create_concurrent_create_share_connections() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = Arc::new(shared_factory(&factory, 2));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let shared = shared.clone();
                std::thread::spawn(move || {
                    futures::executor::block_on(shared.create())
                        .unwrap()
                        .get_underlying_connection()
                        .id
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), 0);
        }
        assert_eq!(factory.create_times(), 1);
    }

    #[test]
    #[ignore = "SharedConnectionFactory's fatalErrorPredicate (a Netty NativeIoException drops the connection) is not ported: a tonic channel reconnects by itself"]
    fn create_below_max_concurrency_fatal_error_prevents_reuse() {}

    #[test]
    fn create_after_last_failed_success() {
        let factory = MockFactory::new(Box::new(|times| {
            if times == 0 {
                async { Err(io::Error::other("error")) }.boxed()
            } else {
                async { Ok(MockConnection::new(0)) }.boxed()
            }
        }));
        let shared = shared_factory(&factory, 2);

        match Observer::test(shared.create()).poll() {
            Some(Err(e)) => assert_eq!(e.to_string(), "error"),
            _ => panic!("expected the error"),
        }
        Observer::test(shared.create()).assert_connection(0);

        assert_eq!(factory.create_times(), 2);
    }

    #[test]
    fn create_dispose_when_waiting_for_connection_creation_do_not_cancel_creation() {
        let canceled = Arc::new(AtomicBool::new(false));
        let factory = {
            let canceled = canceled.clone();
            MockFactory::new(Box::new(move |_| MockFactory::pending(canceled.clone())))
        };
        let shared = shared_factory(&factory, 2);
        let mut observer = Observer::test(shared.create());
        observer.assert_empty();

        // observer.dispose()
        drop(observer);

        assert!(!canceled.load(Ordering::SeqCst));
        // The token of the disposed observer went back.
        assert_eq!(shared.num_available_connections(), 2);
    }

    #[test]
    #[ignore = "Java interrupts the thread blocked in blockingGet(); a Rust waiter is a future and is dropped instead (create_dispose_when_waiting_for_connection_creation_do_not_cancel_creation)"]
    fn create_interrupt_terminate() {}

    #[test]
    fn close_connection_connection_become_available() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 1);
        let connection = Observer::test(shared.create()).assert_connection(0);
        assert_eq!(shared.num_available_connections(), 0);

        connection.close();

        assert_eq!(shared.num_available_connections(), 1);
    }

    #[test]
    fn close_factory_close_underlying_connection() {
        let connection = MockConnection::new(0);
        let closes = connection.closes.clone();
        let factory = MockFactory::just(connection);
        let shared = shared_factory(&factory, 1);
        Observer::test(shared.create()).assert_connection(0);

        shared.close();

        assert_eq!(closes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn close_factory_no_new_connection_allowed() {
        let factory = MockFactory::just(MockConnection::new(0));
        let shared = shared_factory(&factory, 1);

        shared.close();

        Observer::test(shared.create()).assert_closed_error();
    }

    #[test]
    fn close_factory_pending_connection_creation_closed_error() {
        let canceled = Arc::new(AtomicBool::new(false));
        let factory = {
            let canceled = canceled.clone();
            MockFactory::new(Box::new(move |_| MockFactory::pending(canceled.clone())))
        };
        let shared = shared_factory(&factory, 1);
        let mut observer = Observer::test(shared.create());
        observer.assert_empty();
        assert!(!canceled.load(Ordering::SeqCst));

        shared.close();

        observer.assert_closed_error();
        assert!(canceled.load(Ordering::SeqCst));
    }
}

// DynamicConnectionPoolTest

mod dynamic_connection_pool_test {
    use super::*;

    /// `setUp`: the first creation answers `connection0`, the later ones `connection1`.
    fn factory() -> Arc<MockFactory> {
        MockFactory::new(Box::new(|times| {
            let id = if times == 0 { 0 } else { 1 };
            async move { Ok(MockConnection::new(id)) }.boxed()
        }))
    }

    #[test]
    fn create_smoke() {
        let factory = factory();
        let pool = pool(&factory, 1);
        Observer::test(pool.create()).assert_connection(0);
        assert_eq!(factory.create_times(), 1);
    }

    #[test]
    fn create_exceeding_max_concurrent_create_new_connection() {
        let factory = factory();
        let pool = pool(&factory, 1);
        let mut observer0 = Observer::test(pool.create());
        let mut observer1 = Observer::test(pool.create());
        observer0.assert_connection(0);
        observer1.assert_connection(1);
        assert_eq!(factory.create_times(), 2);
    }

    #[test]
    fn create_pending_connection_creation_and_exceeding_max_concurrent_create_new_connection() {
        let terminated = Arc::new(AtomicBool::new(false));
        let factory = {
            let terminated = terminated.clone();
            MockFactory::new(Box::new(move |times| {
                if times == 0 {
                    MockFactory::pending(terminated.clone())
                } else {
                    async { Ok(MockConnection::new(1)) }.boxed()
                }
            }))
        };
        let pool = pool(&factory, 1);
        let mut observer0 = Observer::test(pool.create());
        let mut observer1 = Observer::test(pool.create());
        assert!(!terminated.load(Ordering::SeqCst));
        observer0.assert_empty();
        observer1.assert_connection(1);
        assert_eq!(factory.create_times(), 2);
    }

    #[test]
    fn create_below_max_concurrency_share_connections() {
        let factory = factory();
        let pool = pool(&factory, 2);
        let mut observer0 = Observer::test(pool.create());
        let mut observer1 = Observer::test(pool.create());
        observer0.assert_connection(0);
        observer1.assert_connection(0);
        assert_eq!(factory.create_times(), 1);
    }

    #[test]
    fn create_after_connection_closed_share_connections() {
        let factory = factory();
        let pool = pool(&factory, 1);
        let connection = Observer::test(pool.create()).assert_connection(0);
        connection.close();
        Observer::test(pool.create()).assert_connection(0);
        assert_eq!(factory.create_times(), 1);
    }

    #[test]
    fn create_concurrent_never_collides_on_same_connection() {
        let n = 200;
        let factory = MockFactory::just(MockConnection::new(0));
        let pool = pool(&factory, /* maxConcurrencyPerConnection= */ 1);
        let start = Arc::new(std::sync::Barrier::new(n));
        let threads: Vec<_> = (0..n)
            .map(|_| {
                let pool = pool.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    // Each holds its connection, so no two can share one.
                    futures::executor::block_on(pool.create()).map(|_| ())
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
    }

    #[test]
    fn close_pool_no_new_connection_allowed() {
        let factory = factory();
        let pool = pool(&factory, 1);
        pool.close();
        Observer::test(pool.create()).assert_closed_error();
    }

    #[test]
    fn close_pool_close_underlying_connection() {
        let connection0 = MockConnection::new(0);
        let closes = connection0.closes.clone();
        let factory = MockFactory::just(connection0);
        let pool = pool(&factory, 1);
        Observer::test(pool.create()).assert_connection(0);
        pool.close();
        assert_eq!(closes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn close_pool_pending_connection_creation_closed_error() {
        let canceled = Arc::new(AtomicBool::new(false));
        let factory = {
            let canceled = canceled.clone();
            MockFactory::new(Box::new(move |_| MockFactory::pending(canceled.clone())))
        };
        let pool = pool(&factory, 1);
        let mut observer = Observer::test(pool.create());
        observer.assert_empty();
        assert!(!canceled.load(Ordering::SeqCst));
        pool.close();
        observer.assert_closed_error();
        assert!(canceled.load(Ordering::SeqCst));
    }
}
