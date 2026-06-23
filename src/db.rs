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

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;

        Ok(Self { conn })
    }

    pub fn default_path() -> anyhow::Result<std::path::PathBuf> {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("orbit");

        Ok(base.join("orbit.db"))
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
