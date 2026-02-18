/// A local executor for hyper that uses `spawn_local` instead of `spawn`.
///
/// Required for HTTP/2 because hyper needs an executor for background tasks,
/// and dpdk-net's streams are `!Send`.
#[derive(Clone, Copy)]
pub struct LocalExecutor;

impl<F> hyper::rt::Executor<F> for LocalExecutor
where
    F: std::future::Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        tokio::task::spawn_local(fut);
    }
}
