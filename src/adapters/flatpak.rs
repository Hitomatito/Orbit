use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, DependencyType, InstallScope, IntegrityStatus, PackageSource,
    ProcessInfo, SizeBreakdown, StageType, SystemPath, UninstallOptions, UninstallPlan,
    UninstallResult, UninstallStage, UninstallWarning,
};

pub struct FlatpakAdapter;

impl FlatpakAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn flatpak_output(args: &[&str]) -> Result<Vec<String>, AdapterError> {
        let output = Command::new("flatpak")
            .env("LC_ALL", "C")
            .args(args)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdapterError::Backend(format!(
                "flatpak failed: {}",
                stderr
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }

    /// Parse flatpak's human-readable size like "384.3 MB" into bytes.
    /// Flatpak uses SI units (MB = 10^6) with locale-dependent decimal separator.
    fn parse_size(s: &str) -> u64 {
        let s = s.trim().replace('\u{00a0}', " ").replace('\u{202f}', " ");
        let mut parts = s.split_whitespace();
        let num_str = match parts.next() {
            Some(n) => n,
            None => return 0,
        };
        let unit = parts.next().unwrap_or("MB");

        let num: f64 = match num_str.parse() {
            Ok(n) => n,
            Err(_) => {
                // Try with comma as decimal separator (e.g. "384,3")
                let fixed = num_str.replace(',', ".");
                match fixed.parse() {
                    Ok(n) => n,
                    Err(_) => return 0,
                }
            }
        };

        let multiplier: u64 = match unit {
            "B" => 1,
            "KB" | "kB" => 1000,
            "MB" => 1_000_000,
            "GB" => 1_000_000_000,
            "TB" => 1_000_000_000_000,
            _ => 1,
        };

        (num * multiplier as f64) as u64
    }

    /// Generate the probable install directory for a flatpak app.
    fn install_base(app_id: &str, installation: &str) -> String {
        let base = match installation {
            "user" => format!(
                "{}/.local/share/flatpak/app/{}",
                std::env::var("HOME").unwrap_or_else(|_| "/root".into()),
                app_id
            ),
            _ => format!("/var/lib/flatpak/app/{}", app_id),
        };

        // Try current/active/files first (modern flatpak), then active/files
        let candidates = vec![
            format!("{}/current/active/files", base),
            format!("{}/active/files", base),
        ];

        for path in &candidates {
            if std::path::Path::new(path).exists() {
                return path.clone();
            }
        }
        // Return the most likely path anyway (will fail gracefully in du)
        candidates.into_iter().next().unwrap()
    }

    /// Measure the actual installed size of a Flatpak app via du.
    async fn measure_footprint(app_id: &str, installation: &str) -> SizeBreakdown {
        let files_dir = Self::install_base(app_id, installation);

        let total_footprint = Command::new("du")
            .args(["-sb", &files_dir])
            .output()
            .await
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout);
                    s.split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                } else {
                    None
                }
            })
            .unwrap_or(0);

        SizeBreakdown {
            total_footprint,
            ..Default::default()
        }
    }

    async fn flatpak_size(app_id: &str) -> u64 {
        Self::flatpak_output(&["info", "--show-size", app_id])
            .await
            .ok()
            .and_then(|lines| lines.first().and_then(|s| s.trim().parse::<u64>().ok()))
            .unwrap_or(0)
    }

    fn flatpak_var_data(app_id: &str) -> String {
        format!(
            "{}/.var/app/{}",
            std::env::var("HOME").unwrap_or_else(|_| "/root".into()),
            app_id
        )
    }

    async fn is_process_running(binary_hint: &str) -> Vec<ProcessInfo> {
        let Ok(mut entries) = tokio::fs::read_dir("/proc").await else {
            return Vec::new();
        };
        let hint_lower = binary_hint.to_lowercase();
        let mut processes = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let pid_str = entry.file_name().to_string_lossy().to_string();
            let pid: u32 = match pid_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let cmdline_path = format!("/proc/{}/cmdline", pid_str);
            if let Ok(cmd) = tokio::fs::read_to_string(&cmdline_path).await {
                if cmd.to_lowercase().contains(&hint_lower) {
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
impl PackageAdapter for FlatpakAdapter {
    fn backend_id(&self) -> &'static str {
        "flatpak"
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("flatpak")
            .arg("--version")
            .output()
            .is_ok()
    }

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError> {
        let lines = Self::flatpak_output(&[
            "list",
            "--app",
            "--columns=application,name,version,branch,arch,installation,size",
        ])
        .await?;

        let mut apps = Vec::new();

        for line in lines {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }
            if line.is_empty() || line.starts_with("Application ID") {
                continue;
            }

            let mut parts = line.split('\t');
            let app_id = match parts.next() {
                Some(id) => id.to_string(),
                None => continue,
            };
            let display_name = parts.next().unwrap_or(&app_id).to_string();
            let version = parts.next().unwrap_or("").to_string();
            let _branch = parts.next().unwrap_or("").to_string();
            let arch = parts.next().unwrap_or("").to_string();
            let installation = parts.next().unwrap_or("system").to_string();
            let size = parts.next().map(Self::parse_size).unwrap_or(0);

            let size_bytes = if size > 0 {
                SizeBreakdown {
                    total_footprint: size,
                    package_size: size,
                    ..Default::default()
                }
            } else {
                Self::measure_footprint(&app_id, &installation).await
            };

            apps.push(AppFootprint {
                id: format!("flatpak:{}", &app_id),
                display_name,
                source: PackageSource::Flatpak,
                version,
                architecture: arch,
                scope: if installation == "user" {
                    InstallScope::User
                } else {
                    InstallScope::System
                },
                size_bytes,
                ..Default::default()
            });
        }

        Ok(apps)
    }

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError> {
        let fp_id = app_id.strip_prefix("flatpak:").unwrap_or(app_id);

        let system_path = format!("/var/lib/flatpak/app/{}", fp_id);
        let installation = if std::path::Path::new(&system_path).exists() {
            "system"
        } else {
            "user"
        };

        let lines = Self::flatpak_output(&["info", fp_id]).await?;

        // First non-empty line before the table is "<Name> - <summary>"
        let mut display_name = fp_id.to_string();
        let mut summary = String::new();
        let mut version = String::new();
        let mut arch = String::new();
        let mut license = None;
        let mut inside_table = false;

        for line in &lines {
            if line.trim().is_empty() {
                continue;
            }
            if line.trim().starts_with("ID:") {
                inside_table = true;
                continue;
            }
            if !inside_table {
                // First content line: "Name - Summary"
                let dash_pos = line.find(" - ");
                if let Some(pos) = dash_pos {
                    display_name = line[..pos].trim().to_string();
                    summary = line[pos + 3..].trim().to_string();
                } else {
                    display_name = line.trim().to_string();
                }
                continue;
            }
            if let Some(v) = line.strip_prefix("Version:") {
                version = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("Arch:") {
                arch = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("License:") {
                let l = v.trim().to_string();
                if !l.is_empty() {
                    license = Some(l);
                }
            }
        }

        // Exact size via --show-size
        let size_from_info = Self::flatpak_output(&["info", "--show-size", fp_id])
            .await
            .ok()
            .and_then(|lines| {
                lines
                    .first()
                    .and_then(|s| s.trim().parse::<u64>().ok())
            })
            .unwrap_or(0);

        let size_bytes = if size_from_info > 0 {
            SizeBreakdown {
                total_footprint: size_from_info,
                package_size: size_from_info,
                ..Default::default()
            }
        } else {
            Self::measure_footprint(fp_id, installation).await
        };

        Ok(AppFootprint {
            id: format!("flatpak:{}", fp_id),
            display_name,
            source: PackageSource::Flatpak,
            version,
            architecture: arch,
            scope: if installation == "user" {
                InstallScope::User
            } else {
                InstallScope::System
            },
            size_bytes,
            summary,
            license,
            ..Default::default()
        })
    }

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let fp_id = app_id.strip_prefix("flatpak:").unwrap_or(app_id);

        // Try flatpak info --show-files first; if not supported, walk the filesystem
        match Self::flatpak_output(&["info", "--show-files", fp_id]).await {
            Ok(lines) => {
                let files: Vec<_> = lines.into_iter().filter(|l| !l.is_empty()).collect();
                if !files.is_empty() {
                    return Ok(files);
                }
            }
            Err(_) => {}
        }

        // Fallback: list files under the install directory
        let system_path = format!("/var/lib/flatpak/app/{}", fp_id);
        let base = if std::path::Path::new(&system_path).exists() {
            system_path
        } else {
            format!(
                "{}/.local/share/flatpak/app/{}",
                std::env::var("HOME").unwrap_or_else(|_| "/root".into()),
                fp_id
            )
        };

        let files_dir = format!("{}/current/active/files", base);
        let mut files = Vec::new();

        if std::path::Path::new(&files_dir).is_dir() {
            for entry in walkdir::WalkDir::new(&files_dir).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    files.push(entry.path().to_string_lossy().to_string());
                }
            }
        }

        Ok(files)
    }

    async fn list_declared_configs(&self, _app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        Ok(Vec::new())
    }

    async fn list_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError> {
        let fp_id = app_id.strip_prefix("flatpak:").unwrap_or(app_id);

        // --show-dependencies requires flatpak >= 1.14.6, but may not be available.
        // Fallback to --show-runtime for the primary runtime dependency.
        let runtime = Self::flatpak_output(&["info", "--show-runtime", fp_id])
            .await
            .ok()
            .and_then(|lines| lines.first().cloned());

        let mut deps = Vec::new();
        if let Some(r) = runtime {
            if !r.is_empty() {
                deps.push(DependencyInfo {
                    name: r,
                    version: String::new(),
                    dependency_type: DependencyType::Runtime,
                });
            }
        }

        Ok(deps)
    }

    async fn list_reverse_dependencies(
        &self,
        _app_id: &str,
    ) -> Result<Vec<String>, AdapterError> {
        Err(AdapterError::NotAvailable(
            "reverse dependencies not available for flatpak".into(),
        ))
    }

    async fn check_integrity(&self, _app_id: &str) -> Result<IntegrityStatus, AdapterError> {
        Err(AdapterError::NotAvailable(
            "flatpak integrity check not implemented yet".into(),
        ))
    }

    async fn plan_uninstall(
        &self,
        app_id: &str,
        _options: UninstallOptions,
    ) -> Result<UninstallPlan, AdapterError> {
        let fp_id = app_id.strip_prefix("flatpak:").unwrap_or(app_id);

        // Verify app exists
        let info = Self::flatpak_output(&["info", fp_id]).await?;
        let info_line = info.iter()
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| AdapterError::AppNotFound(fp_id.to_string()))?;

        let display_name = info_line.trim().split(" - ").next().unwrap_or(fp_id).to_string();

        // Get the runtime dependency
        let runtime = Self::flatpak_output(&["info", "--show-runtime", fp_id])
            .await
            .ok()
            .and_then(|l| l.first().cloned())
            .unwrap_or_default();

        let size = Self::flatpak_size(fp_id).await;

        let data_dir = Self::flatpak_var_data(fp_id);
        let has_data = std::path::Path::new(&data_dir).exists();

        let mut stages = Vec::new();

        stages.push(UninstallStage {
            stage_type: StageType::RemovePackage,
            description: format!("Remove Flatpak app '{}' (size: {})", display_name, size),
            items: vec![fp_id.to_string()],
            requires_root: false,
            reversible: false,
        });

        if has_data {
            stages.push(UninstallStage {
                stage_type: StageType::RemoveData,
                description: format!("Remove user data at {}", data_dir),
                items: vec![data_dir.clone()],
                requires_root: false,
                reversible: true,
            });
        }

        let mut warnings = Vec::new();
        if !runtime.is_empty() {
            warnings.push(UninstallWarning::SharedDependencies(vec![runtime]));
        }
        let procs = Self::is_process_running(fp_id).await;
        if !procs.is_empty() {
            warnings.push(UninstallWarning::RunningProcesses(procs));
        }

        Ok(UninstallPlan {
            app_id: app_id.to_string(),
            app_name: display_name,
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
        let fp_id = plan.app_id.strip_prefix("flatpak:").unwrap_or(&plan.app_id);

        // Determine if it's a system or user install
        let system_path = format!("/var/lib/flatpak/app/{}", fp_id);
        let is_system = std::path::Path::new(&system_path).exists();

        let mut args = vec!["uninstall", "-y", "--noninteractive"];
        if is_system {
            args.push("--system");
        }
        args.push(fp_id);

        let output = Command::new("flatpak")
            .args(&args)
            .output()
            .await?;

        if output.status.success() {
            // Try to clean up user data directory
            let data_dir = Self::flatpak_var_data(fp_id);
            if std::path::Path::new(&data_dir).exists() {
                let _ = tokio::fs::remove_dir_all(&data_dir).await;
            }

            Ok(UninstallResult {
                app_id: plan.app_id.clone(),
                success: true,
                stages_completed: vec![StageType::RemovePackage],
                stages_failed: Vec::new(),
                space_freed: plan.total_space_to_free,
                backup_path: None,
                error_message: None,
            })
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let err_msg = stderr.trim().to_string();

            // If permission denied, try with pkexec
            if err_msg.contains("permission") || err_msg.contains("denied") || err_msg.contains("root") {
                let pkexec_output = Command::new("pkexec")
                    .args(&["flatpak", "uninstall", "-y", "--noninteractive", fp_id])
                    .output()
                    .await?;

                if pkexec_output.status.success() {
                    return Ok(UninstallResult {
                        app_id: plan.app_id.clone(),
                        success: true,
                        stages_completed: vec![StageType::RemovePackage],
                        stages_failed: Vec::new(),
                        space_freed: plan.total_space_to_free,
                        backup_path: None,
                        error_message: None,
                    });
                }
            }

            Ok(UninstallResult {
                app_id: plan.app_id.clone(),
                success: false,
                stages_completed: Vec::new(),
                stages_failed: vec![StageType::RemovePackage],
                space_freed: 0,
                backup_path: None,
                error_message: Some(err_msg),
            })
        }
    }
}
