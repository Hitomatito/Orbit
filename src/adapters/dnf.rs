use chrono::{TimeZone, Utc};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, DependencyType, InstallScope, IntegrityStatus, PackageSource,
    SizeBreakdown, SystemPath, UninstallOptions, UninstallPlan, UninstallResult,
};

pub struct DnfAdapter;

impl DnfAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn rpm_output(args: &[&str]) -> Result<Vec<String>, AdapterError> {
        let output = Command::new("rpm").args(args).output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdapterError::Backend(format!("rpm failed: {}", stderr)));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }
}

#[async_trait]
impl PackageAdapter for DnfAdapter {
    fn backend_id(&self) -> &'static str {
        "rpm"
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("rpm")
            .arg("--version")
            .output()
            .is_ok()
    }

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError> {
        let lines = Self::rpm_output(&[
            "-qa",
            "--queryformat",
            "%{NAME}\t%{VERSION}-%{RELEASE}\t%{ARCH}\t%{SIZE}\t%{INSTALLTIME}\t%{SUMMARY}\n",
        ])
        .await?;

        let mut apps = Vec::with_capacity(lines.len());

        for line in lines {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }
            if line.is_empty() {
                continue;
            }

            let mut parts = line.split('\t');
            let name = match parts.next() {
                Some(n) => n.to_string(),
                None => continue,
            };
            let version = parts.next().unwrap_or("").to_string();
            let _arch = parts.next().unwrap_or("").to_string();
            let size: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let install_ts: Option<i64> = parts.next().and_then(|s| s.parse().ok());
            let summary = parts.next().unwrap_or("").to_string();

            apps.push(AppFootprint {
                id: format!("rpm:{}", &name),
                display_name: name,
                source: PackageSource::Rpm,
                version,
                architecture: _arch,
                scope: InstallScope::System,
                size_bytes: SizeBreakdown {
                    package_size: size,
                    total_footprint: size,
                    ..Default::default()
                },
                summary,
                installed_at: install_ts.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
                ..Default::default()
            });
        }

        Ok(apps)
    }

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError> {
        let pkg = app_id.strip_prefix("rpm:").unwrap_or(app_id);

        let info_lines = Self::rpm_output(&[
            "-q",
            "--queryformat",
            "%{NAME}\t%{VERSION}-%{RELEASE}\t%{ARCH}\t%{SIZE}\t%{INSTALLTIME}\t%{SUMMARY}\t%{URL}\t%{LICENSE}\n",
            pkg,
        ])
        .await?;

        let info = info_lines
            .first()
            .ok_or_else(|| AdapterError::AppNotFound(pkg.to_string()))?;

        let parts: Vec<&str> = info.splitn(8, '\t').collect();
        if parts.len() < 6 {
            return Err(AdapterError::Parse("unexpected rpm info format".into()));
        }

        let name = parts[0].to_string();
        let version = parts[1].to_string();
        let arch = parts[2].to_string();
        let size: u64 = parts[3].parse().unwrap_or(0);
        let install_ts: Option<i64> = parts[4].parse().ok();
        let summary = parts.get(5).unwrap_or(&"").to_string();
        let homepage = parts.get(6).filter(|s| !s.is_empty()).map(|s| s.to_string());
        let license = parts.get(7).filter(|s| !s.is_empty()).map(|s| s.to_string());

        let files = self.list_files(app_id).await?;
        let configs = self.list_declared_configs(app_id).await?;

        Ok(AppFootprint {
            id: format!("rpm:{}", &name),
            display_name: name,
            source: PackageSource::Rpm,
            version,
            architecture: arch,
            scope: InstallScope::System,
            tracked_files: files,
            declared_configs: configs,
            size_bytes: SizeBreakdown {
                package_size: size,
                total_footprint: size,
                ..Default::default()
            },
            summary,
            homepage,
            license,
            installed_at: install_ts.and_then(|ts| Utc.timestamp_opt(ts, 0).single()),
            ..Default::default()
        })
    }

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let pkg = app_id.strip_prefix("rpm:").unwrap_or(app_id);
        let lines = Self::rpm_output(&["-ql", pkg]).await?;
        Ok(lines.into_iter().filter(|l| !l.is_empty()).collect())
    }

    async fn list_declared_configs(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let pkg = app_id.strip_prefix("rpm:").unwrap_or(app_id);
        let lines = Self::rpm_output(&["-qc", pkg]).await?;
        Ok(lines.into_iter().filter(|l| !l.is_empty()).collect())
    }

    async fn list_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError> {
        let pkg = app_id.strip_prefix("rpm:").unwrap_or(app_id);
        let lines = Self::rpm_output(&["-qR", pkg]).await?;

        let deps = lines
            .into_iter()
            .filter(|l| {
                !l.is_empty() && !l.starts_with("rpmlib(") && !l.contains(".so(")
            })
            .map(|l| {
                let name = l.split_whitespace().next().unwrap_or(&l).to_string();
                DependencyInfo {
                    name,
                    version: String::new(),
                    dependency_type: DependencyType::Required,
                }
            })
            .collect();

        Ok(deps)
    }

    async fn list_reverse_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<String>, AdapterError> {
        let pkg = app_id.strip_prefix("rpm:").unwrap_or(app_id);
        let lines = Self::rpm_output(&["-q", "--whatrequires", pkg]).await?;
        Ok(lines.into_iter().filter(|l| !l.is_empty()).collect())
    }

    async fn check_integrity(&self, _app_id: &str) -> Result<IntegrityStatus, AdapterError> {
        Err(AdapterError::NotAvailable(
            "rpm -V not implemented yet".into(),
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
