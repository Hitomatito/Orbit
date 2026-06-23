use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::adapters::{
    appimage::AppImageAdapter, dnf::DnfAdapter, flatpak::FlatpakAdapter, snap::SnapAdapter,
    PackageAdapter,
};
use crate::db::Database;
use crate::models::{AppFootprint, PackageSource};

pub struct ScanResult {
    pub total: usize,
    pub errors: Vec<(String, String)>,
}

pub struct AppDiscoveryEngine {
    adapters: Vec<Box<dyn PackageAdapter>>,
    db: Arc<Database>,
}

impl AppDiscoveryEngine {
    pub fn new(db: Arc<Database>) -> Self {
        let adapters: Vec<Box<dyn PackageAdapter>> = vec![
            Box::new(DnfAdapter::new()),
            Box::new(FlatpakAdapter::new()),
            Box::new(SnapAdapter::new()),
            Box::new(AppImageAdapter::new()),
        ];

        Self { adapters, db }
    }

    pub async fn scan_all(&self, cancel: CancellationToken) -> ScanResult {
        let mut all_apps = Vec::new();
        let mut errors = Vec::new();

        for adapter in &self.adapters {
            if cancel.is_cancelled() {
                return ScanResult {
                    total: all_apps.len(),
                    errors,
                };
            }

            if !adapter.is_available() {
                continue;
            }

            match adapter.list_installed(cancel.clone()).await {
                Ok(apps) => all_apps.extend(apps),
                Err(e) => errors.push((
                    adapter.backend_id().to_string(),
                    e.to_string(),
                )),
            }
        }

        if let Err(e) = self.db.upsert_apps(&all_apps) {
            errors.push(("database".to_string(), e.to_string()));
        }

        let total = all_apps.len();
        ScanResult { total, errors }
    }

    pub fn get_adapter(&self, source: PackageSource) -> Option<&Box<dyn PackageAdapter>> {
        let backend = match source {
            PackageSource::Rpm => "rpm",
            PackageSource::Apt => "dpkg",
            PackageSource::Flatpak => "flatpak",
            PackageSource::Snap => "snap",
            PackageSource::Loose => "appimage",
            PackageSource::Unknown => return None,
        };
        self.adapters.iter().find(|a| a.backend_id() == backend)
    }

    pub fn get_cached_apps(&self) -> Vec<AppFootprint> {
        self.db.get_all_apps().unwrap_or_default()
    }
}
