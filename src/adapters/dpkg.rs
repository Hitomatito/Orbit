use std::path::Path;

use tokio::process::Command as TokioCmd;
use tokio_util::sync::CancellationToken;

const ACRONYMS: &[&str] = &[
    "3d", "api", "cli", "cpu", "dbus", "diy", "dns", "esr", "gui", "gpu",
    "http", "https", "json", "jwt", "lts", "nvme", "pdf", "php", "png",
    "rest", "rpc", "sdk", "sql", "ssh", "ssl", "tls", "ui", "uri", "url",
    "usb", "utf", "vpn", "xml", "yaml",
];

use async_trait::async_trait;

use crate::adapters::{AdapterError, PackageAdapter};
use crate::models::{
    AppFootprint, DependencyInfo, DependencyType, InstallScope, IntegrityStatus, PackageSource,
    SizeBreakdown, StageType, SystemPath, UninstallOptions, UninstallPlan, UninstallResult,
    UninstallStage, UninstallWarning,
};

pub struct DpkgAdapter;

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut s = bytes as f64;
    let mut i = 0;
    while s >= 1024.0 && i < UNITS.len() - 1 {
        s /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} {}", bytes, UNITS[i])
    } else {
        format!("{:.1} {}", s, UNITS[i])
    }
}

impl DpkgAdapter {
    pub fn new() -> Self {
        Self
    }

    async fn dpkg_query(args: &[&str]) -> Result<Vec<String>, AdapterError> {
        let output = TokioCmd::new("dpkg-query")
            .args(args)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdapterError::Backend(format!(
                "dpkg-query failed: {}",
                stderr
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }

    async fn apt_cache(args: &[&str]) -> Result<Vec<String>, AdapterError> {
        let output = TokioCmd::new("apt-cache")
            .args(args)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdapterError::Backend(format!(
                "apt-cache failed: {}",
                stderr
            )));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }

    fn is_installed(status: &str) -> bool {
        status.contains("installed")
    }

    async fn detailed_footprint(pkg: &str) -> Result<SizeBreakdown, AdapterError> {
        let lines = Self::dpkg_query(&["-L", pkg]).await?;

        let mut total: u64 = 0;
        let mut config: u64 = 0;
        let mut cache: u64 = 0;
        let mut data: u64 = 0;
        let mut shared: u64 = 0;

        for line in &lines {
            if line.is_empty() {
                continue;
            }
            let meta = tokio::fs::metadata(line).await;
            let size = match meta {
                Ok(m) if m.is_file() => m.len(),
                _ => continue,
            };

            total += size;
            let p = Path::new(line);
            let first = p.components().nth(0).map(|c| c.as_os_str().to_string_lossy().to_string());

            match first.as_deref() {
                Some("etc") => config += size,
                Some("var") if line.starts_with("/var/cache/") => cache += size,
                Some("usr") if line.starts_with("/usr/share/")
                    || line.starts_with("/usr/local/share/") => data += size,
                Some("var") if line.starts_with("/var/lib/")
                    || line.starts_with("/var/opt/") => data += size,
                Some("opt") => data += size,
                Some("usr") if line.starts_with("/usr/lib/")
                    || line.starts_with("/usr/lib64/")
                    || line.starts_with("/usr/libexec/") => shared += size,
                _ => {}
            }
        }

        Ok(SizeBreakdown {
            package_size: 0,
            config_size: config,
            cache_size: cache,
            data_size: data,
            shared_size: shared,
            total_footprint: total,
        })
    }

    async fn parse_deplist(dep_field: &str) -> Vec<DependencyInfo> {
        let mut deps = Vec::new();
        for group in dep_field.split(", ") {
            let group = group.trim();
            if group.is_empty() || group == "N/A" {
                continue;
            }
            for alternative in group.split(" | ") {
                let name = alternative
                    .split_whitespace()
                    .next()
                    .unwrap_or(alternative)
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    deps.push(DependencyInfo {
                        name,
                        version: String::new(),
                        dependency_type: DependencyType::Required,
                    });
                }
            }
        }
        deps
    }

    fn humanize_package_name(name: &str) -> String {
        let word = name.split('-')
            .filter(|s| !s.is_empty())
            .map(|s| {
                let lower = s.to_lowercase();
                if ACRONYMS.contains(&lower.as_str()) {
                    lower.to_uppercase()
                } else {
                    let mut c = s.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str().to_lowercase().as_str(),
                    }
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        if word.is_empty() {
            name.to_string()
        } else {
            word
        }
    }

    /// Try to get the appstream display name for a package via `appstreamcli search`.
    /// Returns the first `Nombre:` field found, or None.
    async fn appstream_name(pkg: &str) -> Option<String> {
        let output = TokioCmd::new("appstreamcli")
            .args(["search", pkg])
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Some(val) = line.strip_prefix("Nombre: ") {
                let name = val.trim().to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }

        None
    }
}

#[async_trait]
impl PackageAdapter for DpkgAdapter {
    fn backend_id(&self) -> &'static str {
        "dpkg"
    }

    fn is_available(&self) -> bool {
        std::process::Command::new("dpkg-query")
            .arg("--version")
            .output()
            .is_ok()
    }

    async fn list_installed(
        &self,
        cancel: CancellationToken,
    ) -> Result<Vec<AppFootprint>, AdapterError> {
        let lines = Self::dpkg_query(&[
            "-W",
            "-f",
            "${Package}\t${Version}\t${Architecture}\t${Installed-Size}\t${Status}\t${Description}\n",
        ])
        .await?;

        // First pass: collect installed packages and their names
        let mut pkg_names: Vec<String> = Vec::new();
        let mut raw_entries: Vec<(String, String, String, u64, String)> = Vec::new();

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
            let arch = parts.next().unwrap_or("").to_string();
            let installed_size_kb: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let status = parts.next().unwrap_or("");

            if !Self::is_installed(status) {
                continue;
            }

            let summary = parts.next().unwrap_or("").to_string();
            pkg_names.push(name.clone());
            raw_entries.push((name, version, arch, installed_size_kb, summary));
        }

        // Batch fetch archive sizes via apt-cache show
        let mut archive_sizes: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for chunk in pkg_names.chunks(256) {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }
            let mut cmd_args = vec!["show".to_string()];
            cmd_args.extend(chunk.iter().cloned());
            let cmd_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
            if let Ok(lines) = Self::apt_cache(&cmd_refs).await
            {
                let mut current_pkg = String::new();
                for line in &lines {
                    if let Some(val) = line.strip_prefix("Package: ") {
                        current_pkg = val.trim().to_string();
                    } else if let Some(val) = line.strip_prefix("Size: ") {
                        if let Ok(sz) = val.trim().parse::<u64>() {
                            archive_sizes.insert(current_pkg.clone(), sz);
                        }
                    }
                }
            }
        }

        // Second pass: build AppFootprint entries
        let mut apps = Vec::with_capacity(raw_entries.len());
        for (name, version, arch, installed_size_kb, summary) in raw_entries {
            if cancel.is_cancelled() {
                return Err(AdapterError::Cancelled);
            }

            let total_footprint = installed_size_kb * 1024;
            let package_size = archive_sizes.get(&name).copied().unwrap_or(total_footprint);
            let display_name = Self::humanize_package_name(&name);

            apps.push(AppFootprint {
                id: format!("apt:{}", &name),
                display_name,
                source: PackageSource::Apt,
                version,
                architecture: arch,
                scope: InstallScope::System,
                size_bytes: SizeBreakdown {
                    package_size,
                    total_footprint,
                    ..Default::default()
                },
                summary,
                ..Default::default()
            });
        }

        Ok(apps)
    }

    async fn get_footprint(&self, app_id: &str) -> Result<AppFootprint, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);

        let lines = Self::dpkg_query(&[
            "-W",
            "-f",
            "${Package}\t${Version}\t${Architecture}\t${Installed-Size}\t${Status}\t${Description}\t${Maintainer}\n",
            pkg,
        ])
        .await?;

        let info = lines
            .first()
            .ok_or_else(|| AdapterError::AppNotFound(pkg.to_string()))?;

        let parts: Vec<&str> = info.split('\t').collect();
        if parts.len() < 5 {
            return Err(AdapterError::Parse("unexpected dpkg info format".into()));
        }

        let name = parts[0].to_string();
        let version = parts[1].to_string();
        let arch = parts[2].to_string();
        let installed_size_kb: u64 = parts[3].parse().unwrap_or(0);
        let status = parts[4];
        let summary = parts.get(5).unwrap_or(&"").to_string();

        if !Self::is_installed(status) {
            return Err(AdapterError::AppNotFound(format!("{} not installed", pkg)));
        }

        let files = self.list_files(app_id).await.unwrap_or_default();
        let configs = self.list_declared_configs(app_id).await.unwrap_or_default();

        let mut size_bytes = Self::detailed_footprint(pkg).await.unwrap_or_else(|_| {
            SizeBreakdown {
                package_size: 0,
                total_footprint: installed_size_kb * 1024,
                ..Default::default()
            }
        });
        let dpkg_total = installed_size_kb * 1024;
        if size_bytes.total_footprint < dpkg_total {
            size_bytes.total_footprint = dpkg_total;
        }

        // Fetch homepage, license, and archive size via apt-cache
        let (homepage, license, archive_size) = Self::apt_cache(&["show", pkg])
            .await
            .ok()
            .map(|lines| {
                let mut hp = None;
                let mut lic = None;
                let mut ar_sz = None;
                for l in &lines {
                    if let Some(v) = l.strip_prefix("Homepage: ") {
                        let h = v.trim().to_string();
                        if !h.is_empty() {
                            hp = Some(h);
                        }
                    }
                    if let Some(v) = l.strip_prefix("License: ").or_else(|| l.strip_prefix("Original-License: ")) {
                        let lc = v.trim().to_string();
                        if !lc.is_empty() {
                            lic = Some(lc);
                        }
                    }
                    if let Some(v) = l.strip_prefix("Size: ") {
                        ar_sz = v.trim().parse::<u64>().ok();
                    }
                }
                (hp, lic, ar_sz)
            })
            .unwrap_or((None, None, None));

        if let Some(sz) = archive_size {
            size_bytes.package_size = sz;
        }

        let display_name = Self::appstream_name(pkg)
            .await
            .unwrap_or_else(|| Self::humanize_package_name(&name));

        Ok(AppFootprint {
            id: format!("apt:{}", &name),
            display_name,
            source: PackageSource::Apt,
            version,
            architecture: arch,
            scope: InstallScope::System,
            tracked_files: files,
            declared_configs: configs,
            size_bytes,
            summary,
            homepage,
            license,
            ..Default::default()
        })
    }

    async fn list_files(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);
        let lines = Self::dpkg_query(&["-L", pkg]).await?;
        Ok(lines.into_iter().filter(|l| !l.is_empty()).collect())
    }

    async fn list_declared_configs(&self, app_id: &str) -> Result<Vec<SystemPath>, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);
        // dpkg --get-selections for config files isn't ideal.
        // Use `dpkg-query -W -f='${Conffiles}\n'` format:
        //   /etc/foo.conf abcdef123456...
        match Self::dpkg_query(&["-W", "-f", "${Conffiles}\n", pkg]).await {
            Ok(lines) => {
                let configs: Vec<_> = lines
                    .into_iter()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                    .filter(|p| !p.is_empty())
                    .collect();
                Ok(configs)
            }
            Err(_) => Ok(Vec::new()),
        }
    }

    async fn list_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<DependencyInfo>, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);
        let lines = Self::dpkg_query(&[
            "-W",
            "-f",
            "${Depends}\n${Pre-Depends}\n${Recommends}\n${Suggests}\n",
            pkg,
        ])
        .await?;

        let mut deps = Vec::new();

        for line in &lines {
            if line.is_empty() || line == "N/A" {
                continue;
            }
            // Each line is a dependency field; parse them
            let dep_infos = Self::parse_deplist(line).await;
            deps.extend(dep_infos);
        }

        Ok(deps)
    }

    async fn list_reverse_dependencies(
        &self,
        app_id: &str,
    ) -> Result<Vec<String>, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);

        // Use `apt-cache rdepends --installed` for reverse deps
        let lines = Self::apt_cache(&["rdepends", "--installed", pkg]).await?;

        let rev_deps: Vec<String> = lines
            .into_iter()
            .skip(1)
            .filter(|l| {
                let trimmed = l.trim();
                !trimmed.is_empty()
                    && trimmed != pkg
                    && !trimmed.eq_ignore_ascii_case("Reverse Depends:")
            })
            .map(|l| l.trim().to_string())
            .collect();

        Ok(rev_deps)
    }

    async fn check_integrity(&self, _app_id: &str) -> Result<IntegrityStatus, AdapterError> {
        Err(AdapterError::NotAvailable(
            "dpkg integrity check not implemented yet (debsums)".into(),
        ))
    }

    async fn plan_uninstall(
        &self,
        app_id: &str,
        options: UninstallOptions,
    ) -> Result<UninstallPlan, AdapterError> {
        let pkg = app_id.strip_prefix("apt:").unwrap_or(app_id);

        // Verify the package is installed and get info
        let info = Self::dpkg_query(&[
            "-W",
            "-f",
            "${Package}\t${Version}\t${Installed-Size}\t${Description}\n",
            pkg,
        ])
        .await?;
        let info_line = info
            .first()
            .ok_or_else(|| AdapterError::AppNotFound(pkg.to_string()))?;
        let info_parts: Vec<&str> = info_line.splitn(4, '\t').collect();
        if info_parts.len() < 2 {
            return Err(AdapterError::Parse("unexpected dpkg format".into()));
        }

        let name = info_parts[0].to_string();
        let _version = info_parts[1].to_string();
        let installed_size_kb: u64 = info_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let installed_size = installed_size_kb * 1024;

        let all_files = self.list_files(app_id).await.unwrap_or_default();
        let config_files = self.list_declared_configs(app_id).await.unwrap_or_default();
        let rev_deps = self.list_reverse_dependencies(app_id).await.unwrap_or_default();

        let desktop_entries: Vec<_> = all_files
            .iter()
            .filter(|f| f.ends_with(".desktop"))
            .cloned()
            .collect();

        let mut stages = Vec::new();

        stages.push(UninstallStage {
            stage_type: StageType::RemovePackage,
            description: format!(
                "Remove APT package '{}' (size: {})",
                name,
                format_size(installed_size)
            ),
            items: all_files,
            requires_root: true,
            reversible: false,
        });

        if options.remove_configs && !config_files.is_empty() {
            stages.push(UninstallStage {
                stage_type: StageType::RemoveConfigs,
                description: format!("Remove {} config files", config_files.len()),
                items: config_files,
                requires_root: true,
                reversible: true,
            });
        }

        if !desktop_entries.is_empty() {
            stages.push(UninstallStage {
                stage_type: StageType::RemoveDesktopEntries,
                description: format!("Remove {} desktop entries", desktop_entries.len()),
                items: desktop_entries,
                requires_root: false,
                reversible: true,
            });
        }

        let mut warnings = Vec::new();
        if !rev_deps.is_empty() {
            warnings.push(UninstallWarning::RequiredBySystem(rev_deps));
        }

        Ok(UninstallPlan {
            app_id: app_id.to_string(),
            app_name: name,
            stages,
            total_space_to_free: installed_size,
            warnings,
            backup_recommendation: None,
        })
    }

    async fn execute_uninstall(
        &self,
        plan: &UninstallPlan,
        _cancel: CancellationToken,
    ) -> Result<UninstallResult, AdapterError> {
        let pkg = plan.app_id.strip_prefix("apt:").unwrap_or(&plan.app_id);

        // Try pkexec apt-get remove first; fall back to bare dpkg
        let result = TokioCmd::new("pkexec")
            .args(["apt-get", "remove", "-y", pkg])
            .output()
            .await;

        match result {
            Ok(output) if output.status.success() => {
                Ok(UninstallResult {
                    app_id: plan.app_id.clone(),
                    success: true,
                    stages_completed: vec![StageType::RemovePackage],
                    stages_failed: Vec::new(),
                    space_freed: plan.total_space_to_free,
                    backup_path: None,
                    error_message: None,
                })
            }
            Ok(output) => {
                let pkexec_not_found = output.status.code() == Some(127);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if pkexec_not_found {
                    // Fall back to apt-get directly (will fail without root, but give clear message)
                    let fallback = TokioCmd::new("apt-get")
                        .args(["remove", "-y", pkg])
                        .output()
                        .await?;
                    if fallback.status.success() {
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
                        let err = String::from_utf8_lossy(&fallback.stderr);
                        Ok(UninstallResult {
                            app_id: plan.app_id.clone(),
                            success: false,
                            stages_completed: Vec::new(),
                            stages_failed: vec![StageType::RemovePackage],
                            space_freed: 0,
                            backup_path: None,
                            error_message: Some(format!(
                                "apt-get remove failed: {}. Try with sudo.",
                                err.trim()
                            )),
                        })
                    }
                } else {
                    Ok(UninstallResult {
                        app_id: plan.app_id.clone(),
                        success: false,
                        stages_completed: Vec::new(),
                        stages_failed: vec![StageType::RemovePackage],
                        space_freed: 0,
                        backup_path: None,
                        error_message: Some(format!(
                            "pkexec apt-get remove failed: {}",
                            stderr.trim()
                        )),
                    })
                }
            }
            Err(_e) => {
                // pkexec not installed; try bare apt-get
                let fallback = TokioCmd::new("apt-get")
                    .args(["remove", "-y", pkg])
                    .output()
                    .await?;
                if fallback.status.success() {
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
                    let err = String::from_utf8_lossy(&fallback.stderr);
                    Ok(UninstallResult {
                        app_id: plan.app_id.clone(),
                        success: false,
                        stages_completed: Vec::new(),
                        stages_failed: vec![StageType::RemovePackage],
                        space_freed: 0,
                        backup_path: None,
                        error_message: Some(format!(
                            "apt-get remove failed: {}. Try with sudo.",
                            err.trim()
                        )),
                    })
                }
            }
        }
    }
}
