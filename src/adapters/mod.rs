pub mod dnf;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::models::{
    AppFootprint, DependencyInfo, IntegrityStatus, SystemPath, UninstallOptions, UninstallPlan,
    UninstallResult,
};

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("adapter not available: {0}")]
    NotAvailable(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("app not found: {0}")]
    AppNotFound(String),

    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error: {0}")]
    Parse(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[async_trait]
pub trait PackageAdapter: Send + Sync {
    fn backend_id(&self) -> &'static str;
    fn is_available(&self) -> bool;

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError>;

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError>;

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError>;

    async fn list_declared_configs(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError>;

    async fn list_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError>;

    async fn list_reverse_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<String>, AdapterError>;

    async fn check_integrity(&self, app_id: &str) -> Result<IntegrityStatus, AdapterError>;

    async fn plan_uninstall(
        &self,
        app_id: &str,
        options: UninstallOptions,
    ) -> Result<UninstallPlan, AdapterError>;

    async fn execute_uninstall(
        &self,
        plan: &UninstallPlan,
        cancel: CancellationToken,
    ) -> Result<UninstallResult, AdapterError>;
}
