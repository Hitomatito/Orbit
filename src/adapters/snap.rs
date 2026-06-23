use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, InstallScope, IntegrityStatus, PackageSource, ProcessInfo,
    SizeBreakdown, StageType, SystemPath, UninstallOptions, UninstallPlan, UninstallResult,
    UninstallStage, UninstallWarning,
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

    async fn snap_data_dirs(snap_name: &str) -> Vec<String> {
        let mut dirs = Vec::new();
        let var_snap = format!("/var/snap/{}", snap_name);
        if std::path::Path::new(&var_snap).exists() {
            dirs.push(var_snap);
        }
        let home_snap = format!(
            "{}/snap/{}",
            std::env::var("HOME").unwrap_or_else(|_| "/root".into()),
            snap_name
        );
        if std::path::Path::new(&home_snap).exists() {
            dirs.push(home_snap);
        }
        dirs
    }

    async fn is_snap_running(snap_name: &str) -> Vec<ProcessInfo> {
        let Ok(mut entries) = tokio::fs::read_dir("/proc").await else {
            return Vec::new();
        };
        let name_lower = snap_name.to_lowercase();
        let mut processes = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let pid_str = entry.file_name().to_string_lossy().to_string();
            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let cmdline_path = format!("/proc/{}/cmdline", pid_str);
            if let Ok(cmd) = tokio::fs::read_to_string(&cmdline_path).await {
                if cmd.to_lowercase().contains(&name_lower) {
                    let name = std::path::Path::new(&cmd.split('\0').next().unwrap_or(""))
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    processes.push(ProcessInfo {
                        pid,
                        name,
                        cmdline: cmd.replace('\0', " "),
                        memory_bytes: 0,
                    });
                }
            }
        }
        processes
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
        app_id: &str,
        _options: UninstallOptions,
    ) -> Result<UninstallPlan, AdapterError> {
        let snap_name = app_id.strip_prefix("snap:").unwrap_or(app_id);

        // Verify snap exists
        Self::snap_output(&["list", snap_name]).await?;

        let size = Self::measure_snap_size(snap_name).await;
        let data_dirs = Self::snap_data_dirs(snap_name).await;

        let mut stages = Vec::new();

        stages.push(UninstallStage {
            stage_type: StageType::RemovePackage,
            description: format!("Remove snap '{}' (size: {} bytes)", snap_name, size),
            items: vec![snap_name.to_string()],
            requires_root: true,
            reversible: false,
        });

        if !data_dirs.is_empty() {
            stages.push(UninstallStage {
                stage_type: StageType::RemoveData,
                description: format!("Remove snap data directories ({} items)", data_dirs.len()),
                items: data_dirs.clone(),
                requires_root: true,
                reversible: true,
            });
        }

        let mut warnings = Vec::new();
        let procs = Self::is_snap_running(snap_name).await;
        if !procs.is_empty() {
            warnings.push(UninstallWarning::RunningProcesses(procs));
        }

        Ok(UninstallPlan {
            app_id: app_id.to_string(),
            app_name: snap_name.to_string(),
            stages,
            total_space_to_free: size,
            warnings,
            backup_recommendation: None,
        })
    }

    async fn execute_uninstall(
        &self,
        plan: &UninstallPlan,
        _cancel: CancellationToken,
    ) -> Result<UninstallResult, AdapterError> {
        let snap_name = plan.app_id.strip_prefix("snap:").unwrap_or(&plan.app_id);

        // Try without root first, then use pkexec
        let output = Command::new("snap")
            .args(["remove", snap_name])
            .output()
            .await?;

        let success = if output.status.success() {
            true
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("permission") || stderr.contains("denied") || stderr.contains("root")
            {
                let pkexec_output = Command::new("pkexec")
                    .args(["snap", "remove", snap_name])
                    .output()
                    .await?;
                pkexec_output.status.success()
            } else {
                false
            }
        };

        Ok(UninstallResult {
            app_id: plan.app_id.clone(),
            success,
            stages_completed: if success {
                vec![StageType::RemovePackage]
            } else {
                Vec::new()
            },
            stages_failed: if success {
                Vec::new()
            } else {
                vec![StageType::RemovePackage]
            },
            space_freed: if success { plan.total_space_to_free } else { 0 },
            backup_path: None,
            error_message: if success {
                None
            } else {
                Some("Failed to remove snap package".to_string())
            },
        })
    }
}
