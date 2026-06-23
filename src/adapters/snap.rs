use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, InstallScope, IntegrityStatus, PackageSource,
    SizeBreakdown, SystemPath, UninstallOptions, UninstallPlan, UninstallResult,
};

pub struct SnapAdapter;

impl SnapAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn snap_output(args: &[&str]) -> Result<Vec<String>, AdapterError> {
        let output = Command::new("snap").args(args).output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdapterError::Backend(format!(
                "snap failed: {}",
                stderr
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }

    /// Get the installed size of a snap by measuring its squashfs file.
    async fn measure_snap_size(snap_name: &str) -> u64 {
        // Snaps are stored as squashfs images in /var/lib/snapd/snaps/
        let snap_path = format!("/var/lib/snapd/snaps/{}.snap", snap_name);

        // Try to measure the mounted files too
        let mut total = 0u64;

        // Size of the snap file itself
        if let Ok(meta) = tokio::fs::metadata(&snap_path).await {
            total += meta.len();
        }

        // Also measure the writable areas under /var/snap/
        let var_snap = format!("/var/snap/{}", snap_name);
        if let Ok(mut dir) = tokio::fs::read_dir(&var_snap).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                if let Ok(meta) = entry.metadata().await {
                    if meta.is_dir() {
                        // Use du for directory sizes
                        let out = Command::new("du")
                            .args(["-sb", &entry.path().to_string_lossy().to_string()])
                            .output()
                            .await;
                        if let Ok(o) = out {
                            if o.status.success() {
                                let s = String::from_utf8_lossy(&o.stdout);
                                if let Some(size_str) = s.split_whitespace().next() {
                                    total += size_str.parse::<u64>().unwrap_or(0);
                                }
                            }
                        }
                    } else {
                        total += meta.len();
                    }
                }
            }
        }

        total
    }
}

#[async_trait]
impl PackageAdapter for SnapAdapter {
    fn backend_id(&self) -> &'static str {
        "snap"
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("snap")
            .arg("--version")
            .output()
            .is_ok()
    }

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError> {
        let lines = Self::snap_output(&["list"]).await?;

        let mut apps = Vec::new();

        for line in lines {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }

            // Skip header and empty lines
            if line.is_empty() || line.starts_with("Name ") {
                continue;
            }

            // Format: Name  Version  Rev  Tracking  Publisher  Notes
            let mut parts = line.split_whitespace();
            let name = match parts.next() {
                Some(n) => n.to_string(),
                None => continue,
            };
            let version = parts.next().unwrap_or("").to_string();
            let _rev = parts.next().unwrap_or("").to_string();
            let _tracking = parts.next().unwrap_or("").to_string();
            let _publisher = parts.next().unwrap_or("").to_string();
            let notes = parts.next().unwrap_or("").to_string();

            let size = Self::measure_snap_size(&name).await;

            let notes_str = notes.to_lowercase();
            let scope = if notes_str.contains("classic") {
                InstallScope::System
            } else {
                InstallScope::System
            };

            apps.push(AppFootprint {
                id: format!("snap:{}", &name),
                display_name: name,
                source: PackageSource::Snap,
                version,
                scope,
                size_bytes: SizeBreakdown {
                    package_size: 0,
                    total_footprint: size,
                    ..Default::default()
                },
                ..Default::default()
            });
        }

        Ok(apps)
    }

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError> {
        let snap_name = app_id.strip_prefix("snap:").unwrap_or(app_id);

        let lines = Self::snap_output(&["info", snap_name]).await?;

        let mut version = String::new();
        let mut summary = String::new();
        let mut description = String::new();
        let mut homepage = None;
        let mut license = None;

        for line in &lines {
            if let Some(_v) = line.strip_prefix("name:") {
                // already have name from parameter
            } else if let Some(v) = line.strip_prefix("version:") {
                version = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("summary:") {
                summary = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("description:") {
                description = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("license:") {
                let l = v.trim().to_string();
                if !l.is_empty() && l != "unset" {
                    license = Some(l);
                }
            } else if let Some(v) = line.strip_prefix("contact:") {
                let h = v.trim().to_string();
                if !h.is_empty() {
                    homepage = Some(h);
                }
            }
        }

        let size = Self::measure_snap_size(snap_name).await;

        Ok(AppFootprint {
            id: format!("snap:{}", snap_name),
            display_name: snap_name.to_string(),
            source: PackageSource::Snap,
            version,
            size_bytes: SizeBreakdown {
                package_size: 0,
                total_footprint: size,
                ..Default::default()
            },
            summary,
            description,
            homepage,
            license,
            ..Default::default()
        })
    }

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let snap_name = app_id.strip_prefix("snap:").unwrap_or(app_id);

        // Snaps mount their squashfs at /snap/<name>/current/
        let mount = format!("/snap/{}/current", snap_name);
        let mut files = Vec::new();

        if let Ok(mut dir) = tokio::fs::read_dir(&mount).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let path = entry.path().to_string_lossy().to_string();
                files.push(path);
            }
        }

        Ok(files)
    }

    async fn list_declared_configs(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let snap_name = app_id.strip_prefix("snap:").unwrap_or(app_id);
        let config_dir = format!("/var/snap/{}/current", snap_name);

        let mut configs = Vec::new();
        if let Ok(mut dir) = tokio::fs::read_dir(&config_dir).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                let path = entry.path().to_string_lossy().to_string();
                configs.push(path);
            }
        }

        Ok(configs)
    }

    async fn list_dependencies(
        &self,
        _app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError> {
        // Snaps bundle all dependencies; no standard way to list them
        Ok(Vec::new())
    }

    async fn list_reverse_dependencies(
        &self,
        _app_id: &str,
    ) -> Result<Vec<String>, AdapterError> {
        Err(AdapterError::NotAvailable(
            "snap reverse dependencies not available".into(),
        ))
    }

    async fn check_integrity(&self, _app_id: &str) -> Result<IntegrityStatus, AdapterError> {
        Err(AdapterError::NotAvailable(
            "snap integrity check not implemented yet".into(),
        ))
    }

    async fn plan_uninstall(
        &self,
        _app_id: &str,
        _options: UninstallOptions,
    ) -> Result<UninstallPlan, AdapterError> {
        Err(AdapterError::NotAvailable(
            "uninstall planning not implemented yet".into(),
        ))
    }

    async fn execute_uninstall(
        &self,
        _plan: &UninstallPlan,
        _cancel: CancellationToken,
    ) -> Result<UninstallResult, AdapterError> {
        Err(AdapterError::NotAvailable(
            "uninstall execution not implemented yet".into(),
        ))
    }
}
