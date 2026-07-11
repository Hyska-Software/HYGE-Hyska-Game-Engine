//! SQLite-backed content-addressed asset database.

use std::path::{Path, PathBuf};

use hyge_core::result::{HygeError, HygeResult};
use rusqlite::{params, Connection, OptionalExtension};

use crate::asset::AssetId;

const CURRENT_SCHEMA_VERSION: i64 = 1;

/// One content-addressed asset row exposed to editor tooling.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AssetRecord {
    /// BLAKE3 identity.
    pub asset_id: AssetId,
    /// Path stored by the importer.
    pub path: PathBuf,
}

/// One directed dependency edge in the asset graph.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DependencyEdge {
    /// Asset that owns the reference.
    pub parent: AssetId,
    /// Referenced asset.
    pub child: AssetId,
}

/// Content-addressed asset database.
///
/// The database maps BLAKE3 asset ids to cache paths and records dependency
/// edges between assets. `open` creates the SQLite file when it does not
/// exist, enables WAL journaling, and applies pending schema migrations.
pub struct AssetDb {
    db: Connection,
    cache_dir: PathBuf,
}

impl AssetDb {
    /// Opens an asset database at `path`, creating it and applying migrations
    /// when needed.
    ///
    /// WAL mode and foreign key checks are enabled for every connection. The
    /// cache directory is the parent directory of `path`, or the current
    /// directory when `path` has no parent.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when SQLite cannot open the database, WAL mode
    /// cannot be enabled, or schema migration fails.
    pub fn open(path: &Path) -> HygeResult<Self> {
        let mut db = Connection::open(path).map_err(sqlite_error("open asset database"))?;
        db.pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_error("enable WAL journal mode"))?;
        db.pragma_update(None, "foreign_keys", "ON")
            .map_err(sqlite_error("enable foreign keys"))?;
        run_migrations(&mut db)?;

        let cache_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Ok(Self { db, cache_dir })
    }

    /// Returns the cache directory associated with this database.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Looks up the cache path for an asset id.
    ///
    /// Returns `None` when the id is unknown or when the stored row cannot be
    /// decoded as a path.
    pub fn lookup(&self, hash: &AssetId) -> Option<PathBuf> {
        self.db
            .query_row(
                "SELECT path FROM assets WHERE hash = ?1",
                params![hash.as_bytes().as_slice()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(PathBuf::from)
    }

    /// Returns every asset row in deterministic BLAKE3 order.
    pub fn list_assets(&self) -> Vec<AssetRecord> {
        let mut statement = match self
            .db
            .prepare("SELECT hash, path FROM assets ORDER BY hash")
        {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let rows = match statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(Result::ok)
            .filter_map(|(bytes, path)| {
                Some(AssetRecord {
                    asset_id: asset_id_from_blob(&bytes)?,
                    path: PathBuf::from(path),
                })
            })
            .collect()
    }

    /// Returns every dependency edge in deterministic parent/child order.
    pub fn dependency_edges(&self) -> Vec<DependencyEdge> {
        let mut statement = match self.db.prepare(
            "SELECT parent_hash, child_hash FROM dependency_edges \
             ORDER BY parent_hash, child_hash",
        ) {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };
        let rows = match statement.query_map([], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };
        rows.filter_map(Result::ok)
            .filter_map(|(parent, child)| {
                Some(DependencyEdge {
                    parent: asset_id_from_blob(&parent)?,
                    child: asset_id_from_blob(&child)?,
                })
            })
            .collect()
    }

    /// Inserts or replaces a content-addressed asset path.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when SQLite rejects the write.
    pub fn insert(&mut self, hash: &AssetId, path: &Path) -> HygeResult<()> {
        self.db
            .execute(
                "INSERT OR REPLACE INTO assets (hash, path) VALUES (?1, ?2)",
                params![hash.as_bytes().as_slice(), path.to_string_lossy().as_ref()],
            )
            .map_err(sqlite_error("insert asset path"))?;
        Ok(())
    }

    /// Returns assets that `hash` depends on.
    ///
    /// Unknown assets return an empty vector. Corrupt dependency rows with a
    /// non-32-byte hash are ignored because this API is intentionally
    /// non-fallible in the public architecture sketch.
    pub fn dependencies(&self, hash: &AssetId) -> Vec<AssetId> {
        let mut statement = match self.db.prepare(
            "SELECT child_hash FROM dependency_edges \
             WHERE parent_hash = ?1 ORDER BY child_hash",
        ) {
            Ok(statement) => statement,
            Err(_) => return Vec::new(),
        };

        let rows = match statement.query_map(params![hash.as_bytes().as_slice()], |row| {
            row.get::<_, Vec<u8>>(0)
        }) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        };

        rows.filter_map(Result::ok)
            .filter_map(|bytes| asset_id_from_blob(&bytes))
            .collect()
    }

    /// Records that `parent` depends on `child`.
    ///
    /// Duplicate edges are ignored. With foreign keys enabled, both assets
    /// must already exist in the `assets` table.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when SQLite rejects the edge, including missing
    /// parent or child asset rows.
    pub fn add_dependency(&mut self, parent: &AssetId, child: &AssetId) -> HygeResult<()> {
        self.db
            .execute(
                "INSERT OR IGNORE INTO dependency_edges (parent_hash, child_hash) \
                 VALUES (?1, ?2)",
                params![parent.as_bytes().as_slice(), child.as_bytes().as_slice()],
            )
            .map_err(sqlite_error("insert asset dependency"))?;
        Ok(())
    }

    /// Atomically records a batch of asset rows plus dependency edges.
    ///
    /// Every `assets` entry is `INSERT OR REPLACE`d; every edge is
    /// `INSERT OR IGNORE`d (duplicates are no-ops). Both lists are
    /// applied inside a single SQLite transaction so importers never
    /// leave the database in a half-imported state when an error
    /// aborts the run.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError`] when SQLite rejects the transaction,
    /// including the FK constraint on a dependency edge that
    /// references an asset hash not present in `assets`.
    pub fn record_with_dependencies<I, J>(&mut self, assets: I, edges: J) -> HygeResult<()>
    where
        I: IntoIterator<Item = (AssetId, std::path::PathBuf)>,
        J: IntoIterator<Item = (AssetId, AssetId)>,
    {
        let assets: Vec<(AssetId, std::path::PathBuf)> = assets.into_iter().collect();
        let edges: Vec<(AssetId, AssetId)> = edges.into_iter().collect();

        let transaction = self
            .db
            .transaction()
            .map_err(sqlite_error("begin record_with_dependencies transaction"))?;

        {
            let mut insert_stmt = transaction
                .prepare("INSERT OR REPLACE INTO assets (hash, path) VALUES (?1, ?2)")
                .map_err(sqlite_error("prepare batch asset insert"))?;
            for (hash, path) in &assets {
                insert_stmt
                    .execute(params![
                        hash.as_bytes().as_slice(),
                        path.to_string_lossy().as_ref()
                    ])
                    .map_err(sqlite_error("batch insert asset path"))?;
            }
        }

        {
            let mut edge_stmt = transaction
                .prepare(
                    "INSERT OR IGNORE INTO dependency_edges (parent_hash, child_hash) \
                     VALUES (?1, ?2)",
                )
                .map_err(sqlite_error("prepare batch dependency insert"))?;
            for (parent, child) in &edges {
                edge_stmt
                    .execute(params![
                        parent.as_bytes().as_slice(),
                        child.as_bytes().as_slice()
                    ])
                    .map_err(sqlite_error("batch insert dependency edge"))?;
            }
        }

        transaction
            .commit()
            .map_err(sqlite_error("commit record_with_dependencies transaction"))?;
        Ok(())
    }
}

fn run_migrations(db: &mut Connection) -> HygeResult<()> {
    let mut version = schema_version(db)?;
    while version < CURRENT_SCHEMA_VERSION {
        match version + 1 {
            1 => migrate_to_v1(db)?,
            next => {
                return Err(HygeError::unsupported(format!(
                    "asset database schema version {next}"
                )));
            }
        }
        version += 1;
    }
    Ok(())
}

fn schema_version(db: &Connection) -> HygeResult<i64> {
    let has_table: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = 'db_version'",
            [],
            |row| row.get(0),
        )
        .map_err(sqlite_error("read schema metadata"))?;

    if has_table == 0 {
        return Ok(0);
    }

    db.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM db_version",
        [],
        |row| row.get(0),
    )
    .map_err(sqlite_error("read asset database schema version"))
}

fn migrate_to_v1(db: &mut Connection) -> HygeResult<()> {
    let transaction = db
        .transaction()
        .map_err(sqlite_error("begin asset database migration"))?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS db_version (
                version INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS assets (
                hash BLOB PRIMARY KEY NOT NULL,
                path TEXT NOT NULL,
                imported_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS dependency_edges (
                parent_hash BLOB NOT NULL,
                child_hash BLOB NOT NULL,
                PRIMARY KEY (parent_hash, child_hash),
                FOREIGN KEY (parent_hash) REFERENCES assets(hash) ON DELETE CASCADE,
                FOREIGN KEY (child_hash) REFERENCES assets(hash) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS dependency_edges_child_idx
                ON dependency_edges(child_hash);

            INSERT INTO db_version (version) VALUES (1);",
        )
        .map_err(sqlite_error("apply asset database schema v1"))?;
    transaction
        .commit()
        .map_err(sqlite_error("commit asset database migration"))?;
    Ok(())
}

fn asset_id_from_blob(bytes: &[u8]) -> Option<AssetId> {
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(AssetId::from(bytes))
}

fn sqlite_error(context: &'static str) -> impl FnOnce(rusqlite::Error) -> HygeError {
    move |error| HygeError::parse(format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serial_test::serial;

    use super::*;

    fn test_db_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after UNIX epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "hyge_asset_db_{name}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("test db directory should be created");
        dir.join("assets.sqlite3")
    }

    fn id(bytes: &'static [u8]) -> AssetId {
        AssetId::from(blake3::hash(bytes))
    }

    fn schema_version_for_test(db: &AssetDb) -> i64 {
        db.db
            .query_row("SELECT MAX(version) FROM db_version", [], |row| row.get(0))
            .expect("schema version should be readable")
    }

    fn journal_mode_for_test(db: &AssetDb) -> String {
        db.db
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal mode should be readable")
    }

    #[test]
    #[serial]
    fn open_creates_schema_and_enables_wal() {
        let path = test_db_path("open_creates_schema_and_enables_wal");
        let db = AssetDb::open(&path).expect("asset db should open");

        assert_eq!(schema_version_for_test(&db), CURRENT_SCHEMA_VERSION);
        assert_eq!(journal_mode_for_test(&db), "wal");
        assert_eq!(db.cache_dir(), path.parent().expect("db path has parent"));
    }

    #[test]
    #[serial]
    fn insert_and_lookup_round_trip_path() {
        let path = test_db_path("insert_and_lookup_round_trip_path");
        let mut db = AssetDb::open(&path).expect("asset db should open");
        let asset = id(b"mesh");
        let cache_path = Path::new("cache/meshes/cube.hyge-asset");

        assert_eq!(db.lookup(&asset), None);
        db.insert(&asset, cache_path)
            .expect("asset path should insert");

        assert_eq!(db.lookup(&asset), Some(cache_path.to_path_buf()));
    }

    #[test]
    #[serial]
    fn insert_replaces_existing_path() {
        let path = test_db_path("insert_replaces_existing_path");
        let mut db = AssetDb::open(&path).expect("asset db should open");
        let asset = id(b"texture");

        db.insert(&asset, Path::new("old.hyge-asset"))
            .expect("initial asset path should insert");
        db.insert(&asset, Path::new("new.hyge-asset"))
            .expect("replacement asset path should insert");

        assert_eq!(db.lookup(&asset), Some(PathBuf::from("new.hyge-asset")));
    }

    #[test]
    #[serial]
    fn dependencies_round_trip_and_deduplicate_edges() {
        let path = test_db_path("dependencies_round_trip_and_deduplicate_edges");
        let mut db = AssetDb::open(&path).expect("asset db should open");
        let parent = id(b"material");
        let child = id(b"texture-albedo");

        db.insert(&parent, Path::new("material.hyge-asset"))
            .expect("parent asset should insert");
        db.insert(&child, Path::new("texture.hyge-asset"))
            .expect("child asset should insert");
        db.add_dependency(&parent, &child)
            .expect("dependency should insert");
        db.add_dependency(&parent, &child)
            .expect("duplicate dependency should be ignored");

        assert_eq!(db.dependencies(&parent), vec![child]);
        assert_eq!(db.dependencies(&child), Vec::<AssetId>::new());
    }

    #[test]
    #[serial]
    fn missing_dependency_endpoint_is_rejected() {
        let path = test_db_path("missing_dependency_endpoint_is_rejected");
        let mut db = AssetDb::open(&path).expect("asset db should open");
        let parent = id(b"parent");
        let child = id(b"missing-child");

        db.insert(&parent, Path::new("parent.hyge-asset"))
            .expect("parent asset should insert");

        let error = db
            .add_dependency(&parent, &child)
            .expect_err("foreign key should reject missing child");
        assert!(matches!(error, HygeError::Parse(_)));
    }

    #[test]
    #[serial]
    fn open_existing_database_keeps_schema_version() {
        let path = test_db_path("open_existing_database_keeps_schema_version");
        {
            let mut db = AssetDb::open(&path).expect("asset db should open");
            db.insert(&id(b"asset"), Path::new("asset.hyge-asset"))
                .expect("asset path should insert");
        }

        let db = AssetDb::open(&path).expect("existing asset db should reopen");

        assert_eq!(schema_version_for_test(&db), CURRENT_SCHEMA_VERSION);
        assert_eq!(
            db.lookup(&id(b"asset")),
            Some(PathBuf::from("asset.hyge-asset"))
        );
    }
}
