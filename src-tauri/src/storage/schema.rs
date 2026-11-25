use anyhow::Result;
use rusqlite::Connection;

/// Bump when a new migration step is added.
pub const SCHEMA_VERSION: i32 = 1;

/// Complete schema for a fresh database. Every statement must stay idempotent.
const BASELINE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    app_name TEXT NOT NULL DEFAULT '',
    app_type TEXT NOT NULL DEFAULT '',
    context_profile_id TEXT NOT NULL DEFAULT 'general.native',
    context_label TEXT NOT NULL DEFAULT 'General',
    context_icon_key TEXT NOT NULL DEFAULT 'general',
    context_family TEXT NOT NULL DEFAULT 'general',
    browser_access_status TEXT,
    provider_kind TEXT NOT NULL DEFAULT 'local',
    raw_text TEXT NOT NULL DEFAULT '',
    polished_text TEXT NOT NULL DEFAULT '',
    language TEXT,
    duration_ms INTEGER,
    active_scene_id TEXT,
    active_scene_source TEXT,
    active_scene_name TEXT,
    active_scene_prompt_chars INTEGER,
    active_scene_prompt_truncated INTEGER NOT NULL DEFAULT 0,
    output_status TEXT,
    output_error TEXT
);
CREATE TABLE IF NOT EXISTS dictionary (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    word TEXT NOT NULL,
    pronunciation TEXT
);
CREATE TABLE IF NOT EXISTS correction_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern TEXT NOT NULL,
    replacement TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

/// Columns added after the original history table shipped. They are applied
/// only when missing so databases created before `user_version` was tracked
/// are upgraded in place.
const HISTORY_OPTIONAL_COLUMNS: &[(&str, &str)] = &[
    (
        "active_scene_id",
        "ALTER TABLE history ADD COLUMN active_scene_id TEXT",
    ),
    (
        "active_scene_source",
        "ALTER TABLE history ADD COLUMN active_scene_source TEXT",
    ),
    (
        "active_scene_name",
        "ALTER TABLE history ADD COLUMN active_scene_name TEXT",
    ),
    (
        "active_scene_prompt_chars",
        "ALTER TABLE history ADD COLUMN active_scene_prompt_chars INTEGER",
    ),
    (
        "active_scene_prompt_truncated",
        "ALTER TABLE history ADD COLUMN active_scene_prompt_truncated INTEGER NOT NULL DEFAULT 0",
    ),
    (
        "output_status",
        "ALTER TABLE history ADD COLUMN output_status TEXT",
    ),
    (
        "output_error",
        "ALTER TABLE history ADD COLUMN output_error TEXT",
    ),
    (
        "context_profile_id",
        "ALTER TABLE history ADD COLUMN context_profile_id TEXT NOT NULL DEFAULT 'general.native'",
    ),
    (
        "context_label",
        "ALTER TABLE history ADD COLUMN context_label TEXT NOT NULL DEFAULT 'General'",
    ),
    (
        "context_icon_key",
        "ALTER TABLE history ADD COLUMN context_icon_key TEXT NOT NULL DEFAULT 'general'",
    ),
    (
        "context_family",
        "ALTER TABLE history ADD COLUMN context_family TEXT NOT NULL DEFAULT 'general'",
    ),
    (
        "browser_access_status",
        "ALTER TABLE history ADD COLUMN browser_access_status TEXT",
    ),
    (
        "provider_kind",
        "ALTER TABLE history ADD COLUMN provider_kind TEXT NOT NULL DEFAULT 'local'",
    ),
];

pub fn apply(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(BASELINE_SCHEMA)?;

    let version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version < SCHEMA_VERSION {
        if version < 1 {
            upgrade_to_v1(conn)?;
        }
        // Future steps go here:
        // if version < 2 { upgrade_to_v2(conn)?; }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn upgrade_to_v1(conn: &Connection) -> Result<()> {
    let mut statement = conn.prepare("PRAGMA table_info(history)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<std::collections::HashSet<_>, _>>()?;

    for &(name, ddl) in HISTORY_OPTIONAL_COLUMNS {
        if !columns.contains(name) {
            conn.execute(ddl, [])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn table_columns(conn: &Connection, table: &str) -> HashSet<String> {
        let mut statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        columns
    }

    fn schema_version(conn: &Connection) -> i32 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn apply_is_idempotent_and_records_the_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn).unwrap();
        apply(&conn).unwrap();

        assert_eq!(schema_version(&conn), SCHEMA_VERSION);
        let columns = table_columns(&conn, "history");
        for &(name, _) in HISTORY_OPTIONAL_COLUMNS {
            assert!(columns.contains(name), "missing column {name}");
        }
        for table in ["history", "dictionary", "correction_rules"] {
            assert!(
                !table_columns(&conn, table).is_empty(),
                "missing table {table}"
            );
        }
    }

    #[test]
    fn legacy_history_table_is_upgraded_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                app_name TEXT NOT NULL,
                app_type TEXT NOT NULL,
                raw_text TEXT NOT NULL,
                polished_text TEXT NOT NULL,
                language TEXT,
                duration_ms INTEGER,
                stt_provider TEXT,
                llm_provider TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO history (app_name, app_type, raw_text, polished_text)
             VALUES ('Cursor', 'Editor', 'raw', 'polished')",
            [],
        )
        .unwrap();

        apply(&conn).unwrap();

        assert_eq!(schema_version(&conn), SCHEMA_VERSION);
        let columns = table_columns(&conn, "history");
        assert!(columns.contains("provider_kind"));
        assert!(columns.contains("context_family"));
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
    }
}
