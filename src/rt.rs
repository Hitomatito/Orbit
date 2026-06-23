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
    /// Use with glib::MainContext::spawn() to process results on the GTK thread.
    pub fn spawn_task<F, T>(&self, future: F) -> tokio::task::JoinHandle<T>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        self.tokio.spawn(future)
    }
}
