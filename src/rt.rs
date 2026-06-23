use tokio::runtime::Runtime;

/// Bridge between the Tokio async runtime and the GTK main loop.
/// Tokio handles all I/O and CPU-intensive work on worker threads.
/// Results are dispatched to the GTK thread via glib::MainContext.
pub struct AsyncRuntime {
    tokio: Runtime,
}

impl AsyncRuntime {
    pub fn new() -> anyhow::Result<Self> {
        let tokio = Runtime::new()?;
        Ok(Self { tokio })
    }

    /// Spawn a future on the Tokio runtime.
    /// `on_done` is invoked on the GTK main thread when the future completes.
    pub fn spawn<F, T>(&self, future: F, on_done: impl FnOnce(T) + Send + 'static)
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let ctx = glib::MainContext::default();
        self.tokio.spawn(async move {
            let output = future.await;
            ctx.invoke(move || on_done(output));
        });
    }

    /// Spawn a fallible future. `on_ok`/`on_err` run on the GTK thread.
    pub fn spawn_fallible<F, T, E>(
        &self,
        future: F,
        on_ok: impl FnOnce(T) + Send + 'static,
        on_err: impl FnOnce(E) + Send + 'static,
    ) where
        F: std::future::Future<Output = Result<T, E>> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        let ctx = glib::MainContext::default();
        self.tokio.spawn(async move {
            match future.await {
                Ok(val) => ctx.invoke(move || on_ok(val)),
                Err(err) => ctx.invoke(move || on_err(err)),
            }
        });
    }
}
