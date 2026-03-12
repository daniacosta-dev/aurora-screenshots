use rusqlite::{Connection, Result, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub content: String,
    pub thumbnail: Option<String>,
    pub created_at: String,
}

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            type        TEXT    NOT NULL CHECK(type IN ('text', 'image')),
            content     TEXT    NOT NULL,
            thumbnail   TEXT,
            created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at DESC);",
    )
}

pub fn insert_entry(
    conn: &Connection,
    entry_type: &str,
    content: &str,
    thumbnail: Option<&str>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO history (type, content, thumbnail) VALUES (?1, ?2, ?3)",
        params![entry_type, content, thumbnail],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_entries(conn: &Connection, limit: i64) -> Result<Vec<HistoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, type, content, thumbnail, created_at
         FROM history
         ORDER BY created_at DESC
         LIMIT ?1",
    )?;

    let entries = stmt.query_map(params![limit], |row| {
        Ok(HistoryEntry {
            id: row.get(0)?,
            entry_type: row.get(1)?,
            content: row.get(2)?,
            thumbnail: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;

    entries.collect()
}

pub fn get_entry_by_id(conn: &Connection, id: i64) -> Result<HistoryEntry> {
    conn.query_row(
        "SELECT id, type, content, thumbnail, created_at FROM history WHERE id = ?1",
        params![id],
        |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                entry_type: row.get(1)?,
                content: row.get(2)?,
                thumbnail: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )
}

pub fn delete_entry(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM history WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn clear_entries(conn: &Connection) -> Result<()> {
    conn.execute_batch("DELETE FROM history;")?;
    Ok(())
}
