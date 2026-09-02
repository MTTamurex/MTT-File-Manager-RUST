use super::{backfill_organizer_path_blobs, Connection};

const OPERATIONS_COLUMNS: &str = r#"(
    operation_id TEXT PRIMARY KEY NOT NULL
        CHECK(CAST(operation_id AS INTEGER) > 0
              AND printf('%lld', CAST(operation_id AS INTEGER)) = operation_id),
    rule_id INTEGER,
    source_path TEXT NOT NULL COLLATE NOCASE,
    destination_path TEXT NOT NULL COLLATE NOCASE,
    operation_type TEXT NOT NULL DEFAULT 'move'
        CHECK(operation_type IN ('move', 'retry', 'undo')),
    status TEXT NOT NULL
        CHECK(status IN ('started', 'completed', 'skipped', 'cancelled', 'failed')),
    owner_id TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    error TEXT,
    conflict_id TEXT,
    original_operation_id TEXT,
    effective_source_path TEXT COLLATE NOCASE,
    effective_destination_path TEXT COLLATE NOCASE,
    source_snapshot_before BLOB,
    destination_snapshot_after BLOB,
    source_path_bytes BLOB,
    destination_path_bytes BLOB,
    effective_source_path_bytes BLOB,
    effective_destination_path_bytes BLOB,
    undone_at INTEGER,
    CHECK((status = 'started' AND finished_at IS NULL)
          OR (status <> 'started' AND finished_at IS NOT NULL))
)"#;

pub(super) fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        &format!("CREATE TABLE IF NOT EXISTS organizer_operations {OPERATIONS_COLUMNS}"),
        [],
    )?;

    for (column, definition) in [
        ("source_path_bytes", "BLOB"),
        ("destination_path_bytes", "BLOB"),
        ("conflict_id", "TEXT"),
        (
            "operation_type",
            "TEXT NOT NULL DEFAULT 'move' CHECK(operation_type IN ('move', 'retry', 'undo'))",
        ),
        ("created_at", "INTEGER NOT NULL DEFAULT 0"),
        ("owner_id", "TEXT NOT NULL DEFAULT ''"),
        ("original_operation_id", "TEXT"),
        ("effective_source_path", "TEXT COLLATE NOCASE"),
        ("effective_destination_path", "TEXT COLLATE NOCASE"),
        ("source_snapshot_before", "BLOB"),
        ("destination_snapshot_after", "BLOB"),
        ("effective_source_path_bytes", "BLOB"),
        ("effective_destination_path_bytes", "BLOB"),
        ("undone_at", "INTEGER"),
    ] {
        add_column_if_missing(conn, column, definition)?;
    }

    backfill_organizer_path_blobs(conn, "organizer_operations", "operation_id")?;
    conn.execute(
        "UPDATE organizer_operations
         SET created_at = started_at
         WHERE created_at = 0",
        [],
    )?;
    conn.execute(
        "UPDATE organizer_operations
         SET effective_source_path = source_path
         WHERE effective_source_path IS NULL",
        [],
    )?;
    conn.execute(
        "UPDATE organizer_operations
         SET effective_destination_path = destination_path
         WHERE effective_destination_path IS NULL",
        [],
    )?;
    conn.execute(
        "UPDATE organizer_operations
         SET effective_source_path_bytes = source_path_bytes
         WHERE effective_source_path_bytes IS NULL",
        [],
    )?;
    conn.execute(
        "UPDATE organizer_operations
         SET effective_destination_path_bytes = destination_path_bytes
         WHERE effective_destination_path_bytes IS NULL",
        [],
    )?;

    if rule_id_is_not_null(conn)? {
        migrate_nullable_rule_id(conn)?;
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS organizer_operation_sequence (
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             next_id INTEGER NOT NULL CHECK(next_id > 0)
         )",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO organizer_operation_sequence (singleton, next_id)
         SELECT 1, COALESCE(MAX(CAST(operation_id AS INTEGER)), 0) + 1
         FROM organizer_operations",
        [],
    )?;

    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS validate_organizer_operation_insert
         BEFORE INSERT ON organizer_operations
         WHEN NEW.operation_id IS NULL
              OR CAST(NEW.operation_id AS INTEGER) <= 0
              OR printf('%lld', CAST(NEW.operation_id AS INTEGER)) <> NEW.operation_id
              OR (NEW.status = 'started') <> (NEW.finished_at IS NULL)
         BEGIN
             SELECT RAISE(ABORT, 'invalid organizer operation');
         END;
         CREATE TRIGGER IF NOT EXISTS validate_organizer_operation_update
         BEFORE UPDATE ON organizer_operations
         WHEN NEW.operation_id IS NULL
              OR CAST(NEW.operation_id AS INTEGER) <= 0
              OR printf('%lld', CAST(NEW.operation_id AS INTEGER)) <> NEW.operation_id
              OR (NEW.status = 'started') <> (NEW.finished_at IS NULL)
         BEGIN
             SELECT RAISE(ABORT, 'invalid organizer operation');
         END;",
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_organizer_operations_started_at
         ON organizer_operations(started_at DESC)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_organizer_operations_created_at
         ON organizer_operations(created_at DESC)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_organizer_operations_original_id
         ON organizer_operations(original_operation_id)",
        [],
    )?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_organizer_operations_one_active_undo
         ON organizer_operations(original_operation_id)
         WHERE operation_type = 'undo' AND status IN ('started', 'completed')",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS organizer_undo_exemptions (
             path_bytes BLOB PRIMARY KEY NOT NULL,
             path TEXT NOT NULL COLLATE NOCASE,
             snapshot BLOB NOT NULL
         )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS organizer_operation_completions (
             operation_id TEXT PRIMARY KEY NOT NULL,
             source_path TEXT NOT NULL COLLATE NOCASE,
             destination_path TEXT NOT NULL COLLATE NOCASE,
             source_path_bytes BLOB NOT NULL,
             destination_path_bytes BLOB NOT NULL,
             destination_snapshot BLOB NOT NULL
         )",
        [],
    )?;
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    column: &str,
    definition: &str,
) -> rusqlite::Result<()> {
    if conn
        .prepare(&format!(
            "SELECT {column} FROM organizer_operations LIMIT 0"
        ))
        .is_err()
    {
        conn.execute(
            &format!("ALTER TABLE organizer_operations ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn rule_id_is_not_null(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT \"notnull\" FROM pragma_table_info('organizer_operations') WHERE name = 'rule_id'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|not_null| not_null != 0)
}

fn migrate_nullable_rule_id(conn: &Connection) -> rusqlite::Result<()> {
    let foreign_keys_enabled = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, bool>(0))
        .unwrap_or(false);
    if foreign_keys_enabled {
        conn.execute_batch("PRAGMA foreign_keys = OFF")?;
    }

    let migration = (|| {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS validate_organizer_operation_insert;
                 DROP TRIGGER IF EXISTS validate_organizer_operation_update;
                 DROP INDEX IF EXISTS idx_organizer_operations_started_at;
                  DROP INDEX IF EXISTS idx_organizer_operations_created_at;
                  DROP INDEX IF EXISTS idx_organizer_operations_original_id;
                  DROP INDEX IF EXISTS idx_organizer_operations_one_active_undo;",
            )?;
            conn.execute(
                &format!("CREATE TABLE organizer_operations_new {OPERATIONS_COLUMNS}"),
                [],
            )?;
            conn.execute(
                "INSERT INTO organizer_operations_new (
                     operation_id, rule_id, source_path, destination_path, operation_type,
                     status, owner_id, created_at, started_at, finished_at, error, conflict_id,
                     original_operation_id, effective_source_path, effective_destination_path,
                     source_snapshot_before, destination_snapshot_after, source_path_bytes,
                     destination_path_bytes, effective_source_path_bytes,
                     effective_destination_path_bytes, undone_at
                 )
                  SELECT operation_id, rule_id, source_path, destination_path, operation_type,
                         status, owner_id,
                        CASE WHEN created_at > 0 THEN created_at ELSE started_at END,
                        started_at, finished_at, error, conflict_id, original_operation_id,
                        COALESCE(effective_source_path, source_path),
                        COALESCE(effective_destination_path, destination_path),
                        source_snapshot_before, destination_snapshot_after, source_path_bytes,
                        destination_path_bytes,
                         COALESCE(effective_source_path_bytes, source_path_bytes),
                         COALESCE(effective_destination_path_bytes, destination_path_bytes),
                         undone_at
                 FROM organizer_operations",
                [],
            )?;
            conn.execute_batch(
                "DROP TABLE organizer_operations;
                 ALTER TABLE organizer_operations_new RENAME TO organizer_operations;",
            )?;
            Ok::<_, rusqlite::Error>(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT"),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    })();

    if foreign_keys_enabled {
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
    }
    migration
}
