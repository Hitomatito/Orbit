use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type SystemPath = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageSource {
    Apt,
    Rpm,
    Flatpak,
    Snap,
    Loose,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallScope {
    System,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    Required,
    Recommended,
    Suggested,
    Runtime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeBreakdown {
    pub package_size: u64,
    pub config_size: u64,
    pub cache_size: u64,
    pub data_size: u64,
    pub shared_size: u64,
    pub total_footprint: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityStatus {
    pub checked_at: DateTime<Utc>,
    pub modified_files: Vec<SystemPath>,
    pub missing_files: Vec<SystemPath>,
    pub checksum_algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxPermission {
    pub permission_type: String,
    pub resource: String,
    pub granted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmdline: String,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyInfo {
    pub name: String,
    pub version: String,
    pub dependency_type: DependencyType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallOptions {
    pub remove_configs: bool,
    pub remove_cache: bool,
    pub remove_data: bool,
    pub remove_orphan_deps: bool,
    pub clean_flatpak_runtimes: bool,
    pub create_backup: bool,
}

impl Default for UninstallOptions {
    fn default() -> Self {
        Self {
            remove_configs: true,
            remove_cache: true,
            remove_data: false,
            remove_orphan_deps: false,
            clean_flatpak_runtimes: false,
            create_backup: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageType {
    StopProcesses,
    StopServices,
    RemovePackage,
    RemoveConfigs,
    RemoveCache,
    RemoveData,
    RemoveDesktopEntries,
    RemoveOrphanDeps,
    CleanFlatpakRuntimes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallStage {
    pub stage_type: StageType,
    pub description: String,
    pub items: Vec<SystemPath>,
    pub requires_root: bool,
    pub reversible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UninstallWarning {
    RunningProcesses(Vec<ProcessInfo>),
    SharedDependencies(Vec<String>),
    ModifiedConfigs(Vec<SystemPath>),
    RequiredBySystem(Vec<String>),
    LargeDataLoss(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecommendation {
    pub path: SystemPath,
    pub estimated_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallPlan {
    pub app_id: String,
    pub app_name: String,
    pub stages: Vec<UninstallStage>,
    pub total_space_to_free: u64,
    pub warnings: Vec<UninstallWarning>,
    pub backup_recommendation: Option<BackupRecommendation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UninstallResult {
    pub app_id: String,
    pub success: bool,
    pub stages_completed: Vec<StageType>,
    pub stages_failed: Vec<StageType>,
    pub space_freed: u64,
    pub backup_path: Option<SystemPath>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrivacyConcern {
    Unsandboxed { risk_level: RiskLevel, description: String },
    FullHomeAccess,
    NetworkAccess,
    MediaDeviceAccess(String),
    LocationAccess,
    Custom { severity: RiskLevel, description: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyReport {
    pub score: u8,
    pub concerns: Vec<PrivacyConcern>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationType {
    Scan,
    Uninstall,
    Clean,
    PermissionChange,
    Freeze,
    Backup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationStatus {
    Pending,
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub operation: OperationType,
    pub app_id: Option<String>,
    pub app_name: Option<String>,
    pub details: serde_json::Value,
    pub status: OperationStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub space_freed: u64,
    pub error: Option<String>,
}

impl Default for AppFootprint {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            source: PackageSource::Unknown,
            version: String::new(),
            architecture: String::new(),
            scope: InstallScope::System,
            tracked_files: Vec::new(),
            declared_configs: Vec::new(),
            discovered_configs: Vec::new(),
            discovered_cache: Vec::new(),
            discovered_data: Vec::new(),
            desktop_entries: Vec::new(),
            systemd_services: Vec::new(),
            running_processes: Vec::new(),
            dependencies: Vec::new(),
            reverse_dependencies: Vec::new(),
            is_orphan_candidate: false,
            size_bytes: SizeBreakdown::default(),
            integrity_status: IntegrityStatus {
                checked_at: Utc::now(),
                modified_files: Vec::new(),
                missing_files: Vec::new(),
                checksum_algorithm: String::new(),
            },
            sandbox_permissions: Vec::new(),
            last_accessed: None,
            installed_at: None,
            icon: None,
            summary: String::new(),
            description: String::new(),
            homepage: None,
            license: None,
        }
    }
}

impl Default for SizeBreakdown {
    fn default() -> Self {
        Self {
            package_size: 0,
            config_size: 0,
            cache_size: 0,
            data_size: 0,
            shared_size: 0,
            total_footprint: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppFootprint {
    pub id: String,
    pub display_name: String,
    pub source: PackageSource,
    pub version: String,
    pub architecture: String,
    pub scope: InstallScope,
    pub tracked_files: Vec<SystemPath>,
    pub declared_configs: Vec<SystemPath>,
    pub discovered_configs: Vec<SystemPath>,
    pub discovered_cache: Vec<SystemPath>,
    pub discovered_data: Vec<SystemPath>,
    pub desktop_entries: Vec<SystemPath>,
    pub systemd_services: Vec<String>,
    pub running_processes: Vec<ProcessInfo>,
    pub dependencies: Vec<String>,
    pub reverse_dependencies: Vec<String>,
    pub is_orphan_candidate: bool,
    pub size_bytes: SizeBreakdown,
    pub integrity_status: IntegrityStatus,
    pub sandbox_permissions: Vec<SandboxPermission>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub installed_at: Option<DateTime<Utc>>,
    pub icon: Option<String>,
    pub summary: String,
    pub description: String,
    pub homepage: Option<String>,
    pub license: Option<String>,
}
