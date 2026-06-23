use tokio::runtime::Runtime;

/// Bridge between the Tokio async runtime and the GTK main loop.
/// Tokio handles all I/O and CPU-intensive work on worker threads.
pub struct AsyncRuntime {
    tokio: Runtime,
}

impl AsyncRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let tokio = Runtime::new()?;
        Ok(Self { tokio })
    }

    /// Spawn a future on the Tokio runtime and return a JoinHandle.
    pub fn spawn_task<F, T>(&self, future: F) -> tokio::task::JoinHandle<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        self.tokio.spawn(future)
    }

    /// Block the current thread until the future completes, running it on Tokio.
    pub fn block_on<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        self.tokio.block_on(future)
    }
}
