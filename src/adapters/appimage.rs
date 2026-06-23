use std::path::Path;

use chrono::{TimeZone, Utc};
use tokio::fs;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, InstallScope, IntegrityStatus, PackageSource, SizeBreakdown,
    SystemPath, UninstallOptions, UninstallPlan, UninstallResult,
};

pub struct AppImageAdapter;

impl AppImageAdapter {
    pub fn new() -> Self {
        Self
    }

    fn scan_dirs() -> Vec<String> {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        vec![
            format!("{}/Applications", home),
            format!("{}/AppImages", home),
            format!("{}/bin", home),
            format!("{}/.local/bin", home),
            "/opt".to_string(),
            "/usr/local/bin".to_string(),
        ]
    }

    async fn is_appimage(path: &Path) -> bool {
        // Extension check is fast and reliable for properly named AppImages
        if let Some(ext) = path.extension() {
            if ext.eq_ignore_ascii_case("AppImage") || ext.eq_ignore_ascii_case("appimage") {
                return true;
            }
        }

        // Fallback: check ELF magic + AppImage signature via `file`
        if let Ok(output) = tokio::process::Command::new("file")
            .arg("--brief")
            .arg(path)
            .output()
            .await
        {
            let out = String::from_utf8_lossy(&output.stdout);
            return out.contains("AppImage") || out.contains("Type 3");
        }

        false
    }

    fn extract_version(filename: &str) -> String {
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);

        for seg in stem.split('-').rev() {
            let seg = seg.trim();
            if seg.starts_with(|c: char| c.is_ascii_digit())
                && seg.contains('.')
                && seg.chars().all(|c| c.is_ascii_digit() || c == '.')
            {
                return seg.to_string();
            }
        }

        String::new()
    }

    fn is_arch_seg(s: &str) -> bool {
        matches!(
            s,
            "x86_64" | "amd64" | "i386" | "i686" | "aarch64" | "arm64" | "armhf"
                | "x64" | "x86" | "linux" | "linux64" | "linux32" | "AppImage"
        )
    }

    fn is_version_seg(s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        if s.starts_with(|c: char| c.is_ascii_digit())
            && s.contains('.')
            && s.chars().all(|c| c.is_ascii_digit() || c == '.')
        {
            return true;
        }
        // Also handle "1.19.21-x64" where x64 is still attached after stripping
        false
    }

    fn parse_name(filename: &str) -> String {
        let stem = Path::new(filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(filename);

        // Split into segments and drop trailing noise segments (version, arch)
        let segments: Vec<&str> = stem.split('-').collect();
        let mut end = segments.len();
        while end > 1 {
            let seg = segments[end - 1].trim();
            if Self::is_arch_seg(seg) || Self::is_version_seg(seg) {
                end -= 1;
            } else {
                break;
            }
        }

        let name = segments[..end].join(" ");
        let name = name.replace('_', " ");

        let name = name
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        if name.is_empty() {
            stem.to_string()
        } else {
            name
        }
    }
}

#[async_trait]
impl PackageAdapter for AppImageAdapter {
    fn backend_id(&self) -> &'static str {
        "appimage"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError> {
        let mut apps = Vec::new();

        for dir in Self::scan_dirs() {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }

            let dir_path = Path::new(&dir);
            if !dir_path.exists() {
                continue;
            }

            let mut read_dir = match fs::read_dir(dir_path).await {
                Ok(d) => d,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = read_dir.next_entry().await {
                if cancel.is_cancelled() {
                    return Err(AdapterError::Cancelled);
                }

                let path = entry.path();
                if !Self::is_appimage(&path).await {
                    continue;
                }

                let filename = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();

                let meta = match entry.metadata().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let size = meta.len();
                let modified = meta.modified().ok();

                let display_name = Self::parse_name(&filename);
                let version = Self::extract_version(&filename);

                apps.push(AppFootprint {
                    id: format!("appimage:{}", &filename),
                    display_name,
                    source: PackageSource::Loose,
                    version,
                    scope: InstallScope::User,
                    tracked_files: vec![path.to_string_lossy().to_string()],
                    size_bytes: SizeBreakdown {
                        package_size: size,
                        total_footprint: size,
                        ..Default::default()
                    },
                    installed_at: modified.and_then(|t| {
                        let duration = t
                            .duration_since(std::time::SystemTime::UNIX_EPOCH)
                            .ok()?;
                        Utc.timestamp_opt(duration.as_secs() as i64, 0).single()
                    }),
                    ..Default::default()
                });
            }
        }

        Ok(apps)
    }

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError> {
        let appimage_name = app_id.strip_prefix("appimage:").unwrap_or(app_id);

        for dir in Self::scan_dirs() {
            let path = Path::new(&dir).join(appimage_name);
            if path.exists() {
                let meta = match fs::metadata(&path).await {
                    Ok(m) => m,
                    Err(e) => return Err(AdapterError::Io(e)),
                };

                let size = meta.len();
                let display_name = Self::parse_name(appimage_name);
                let version = Self::extract_version(appimage_name);

                return Ok(AppFootprint {
                    id: format!("appimage:{}", appimage_name),
                    display_name,
                    source: PackageSource::Loose,
                    version,
                    scope: InstallScope::User,
                    tracked_files: vec![path.to_string_lossy().to_string()],
                    size_bytes: SizeBreakdown {
                        package_size: size,
                        total_footprint: size,
                        ..Default::default()
                    },
                    ..Default::default()
                });
            }
        }

        Err(AdapterError::AppNotFound(appimage_name.to_string()))
    }

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let appimage_name = app_id.strip_prefix("appimage:").unwrap_or(app_id);

        for dir in Self::scan_dirs() {
            let path = Path::new(&dir).join(appimage_name);
            if path.exists() {
                return Ok(vec![path.to_string_lossy().to_string()]);
            }
        }

        Ok(Vec::new())
    }

    async fn list_declared_configs(&self, _app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        Ok(Vec::new())
    }

    async fn list_dependencies(
        &self,
        _app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError> {
        Ok(Vec::new())
    }

    async fn list_reverse_dependencies(
        &self,
        _app_id: &str,
    ) -> Result<Vec<String>, AdapterError> {
        Ok(Vec::new())
    }

    async fn check_integrity(&self, _app_id: &str) -> Result<IntegrityStatus, AdapterError> {
        Err(AdapterError::NotAvailable(
            "appimage integrity check not implemented yet".into(),
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
