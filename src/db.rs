use rusqlite::Connection;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS apps (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('apt', 'rpm', 'flatpak', 'snap', 'loose', 'unknown')),
    version TEXT,
    architecture TEXT,
    scope TEXT CHECK(scope IN ('system', 'user')),
    package_size INTEGER DEFAULT 0,
    config_size INTEGER DEFAULT 0,
    cache_size INTEGER DEFAULT 0,
    data_size INTEGER DEFAULT 0,
    shared_size INTEGER DEFAULT 0,
    total_footprint INTEGER DEFAULT 0,
    is_orphan_candidate INTEGER DEFAULT 0,
    integrity_status TEXT,
    last_accessed INTEGER,
    installed_at INTEGER,
    icon TEXT,
    summary TEXT,
    description TEXT,
    homepage TEXT,
    license TEXT,
    discovered_at INTEGER NOT NULL,
    last_updated INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS app_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    file_type TEXT NOT NULL CHECK(file_type IN (
        'tracked', 'declared_config', 'discovered_config',
        'discovered_cache', 'discovered_data', 'desktop',
        'binary', 'shared_lib', 'service_file'
    )),
    size_bytes INTEGER DEFAULT 0,
    modified_at INTEGER,
    accessed_at INTEGER,
    UNIQUE(app_id, path, file_type)
);

CREATE INDEX IF NOT EXISTS idx_app_files_app_id ON app_files(app_id);
CREATE INDEX IF NOT EXISTS idx_app_files_path ON app_files(path);

CREATE TABLE IF NOT EXISTS dependencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    depends_on TEXT NOT NULL,
    dependency_type TEXT NOT NULL CHECK(dependency_type IN ('required', 'recommended', 'suggested', 'runtime')),
    UNIQUE(app_id, depends_on)
);

CREATE INDEX IF NOT EXISTS idx_dependencies_app ON dependencies(app_id);
CREATE INDEX IF NOT EXISTS idx_dependencies_target ON dependencies(depends_on);

CREATE TABLE IF NOT EXISTS sandbox_permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    app_id TEXT NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    permission_type TEXT NOT NULL,
    resource TEXT NOT NULL,
    granted INTEGER DEFAULT 1,
    UNIQUE(app_id, permission_type, resource)
);

CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_type TEXT NOT NULL CHECK(operation_type IN (
        'scan', 'uninstall', 'clean', 'permission_change', 'freeze', 'backup'
    )),
    app_id TEXT,
    details TEXT,
    status TEXT NOT NULL CHECK(status IN ('pending', 'success', 'failed', 'cancelled')),
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    error_message TEXT,
    space_freed_bytes INTEGER DEFAULT 0
);
";

use crate::models::AppFootprint;

pub struct Database {
    conn: std::sync::Mutex<Connection>,
}

impl Database {
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    pub fn default_path() -> anyhow::Result<std::path::PathBuf> {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("orbit");

        Ok(base.join("orbit.db"))
    }

    pub fn upsert_apps(&self, apps: &[AppFootprint]) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();

        let mut stmt = conn.prepare_cached(
            "INSERT INTO apps (id, display_name, source, version, architecture, scope,
             package_size, config_size, cache_size, data_size, shared_size, total_footprint,
             is_orphan_candidate, summary, description, homepage, license, installed_at,
             discovered_at, last_updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
             ON CONFLICT(id) DO UPDATE SET
             display_name=excluded.display_name, version=excluded.version,
             architecture=excluded.architecture, package_size=excluded.package_size,
             total_footprint=excluded.total_footprint, summary=excluded.summary,
             last_updated=excluded.last_updated",
        )?;

        for app in apps {
            let source_str = match app.source {
                crate::models::PackageSource::Apt => "apt",
                crate::models::PackageSource::Rpm => "rpm",
                crate::models::PackageSource::Flatpak => "flatpak",
                crate::models::PackageSource::Snap => "snap",
                crate::models::PackageSource::Loose => "loose",
                crate::models::PackageSource::Unknown => "unknown",
            };

            let scope_str = match app.scope {
                crate::models::InstallScope::System => "system",
                crate::models::InstallScope::User => "user",
            };

            stmt.execute(rusqlite::params![
                app.id,
                app.display_name,
                source_str,
                app.version,
                app.architecture,
                scope_str,
                app.size_bytes.package_size,
                app.size_bytes.config_size,
                app.size_bytes.cache_size,
                app.size_bytes.data_size,
                app.size_bytes.shared_size,
                app.size_bytes.total_footprint,
                app.is_orphan_candidate as i32,
                app.summary,
                app.description,
                app.homepage,
                app.license,
                app.installed_at.map(|t| t.timestamp()),
                now,
                now,
            ])?;
        }

        Ok(())
    }

    pub fn delete_app(&self, app_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM apps WHERE id = ?1", [app_id])?;
        Ok(())
    }

    pub fn get_all_apps(&self) -> anyhow::Result<Vec<AppFootprint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, display_name, source, version, architecture, scope,
             package_size, config_size, cache_size, data_size, shared_size, total_footprint,
             is_orphan_candidate, summary, description, homepage, license, installed_at
             FROM apps ORDER BY display_name",
        )?;

        let apps = stmt.query_map([], |row| {
            let source_str: String = row.get(2)?;
            let scope_str: String = row.get(5)?;

            Ok(AppFootprint {
                id: row.get(0)?,
                display_name: row.get(1)?,
                source: Self::parse_source(&source_str),
                version: row.get(3)?,
                architecture: row.get(4)?,
                scope: if scope_str == "user" {
                    crate::models::InstallScope::User
                } else {
                    crate::models::InstallScope::System
                },
                size_bytes: crate::models::SizeBreakdown {
                    package_size: row.get(6)?,
                    config_size: row.get(7)?,
                    cache_size: row.get(8)?,
                    data_size: row.get(9)?,
                    shared_size: row.get(10)?,
                    total_footprint: row.get(11)?,
                },
                is_orphan_candidate: row.get::<_, i32>(12)? != 0,
                summary: row.get(13)?,
                description: row.get(14)?,
                homepage: row.get(15)?,
                license: row.get(16)?,
                installed_at: row
                    .get::<_, Option<i64>>(17)?
                    .and_then(|ts| chrono::TimeZone::timestamp_opt(&chrono::Utc, ts, 0).single()),
                ..Default::default()
            })
        })?;

        apps.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn parse_source(s: &str) -> crate::models::PackageSource {
        match s {
            "apt" => crate::models::PackageSource::Apt,
            "rpm" => crate::models::PackageSource::Rpm,
            "flatpak" => crate::models::PackageSource::Flatpak,
            "snap" => crate::models::PackageSource::Snap,
            "loose" => crate::models::PackageSource::Loose,
            _ => crate::models::PackageSource::Unknown,
        }
    }
}
