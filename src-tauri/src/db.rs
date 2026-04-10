use rusqlite::{Connection, Result, params, OptionalExtension};
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
        CREATE INDEX IF NOT EXISTS idx_history_created_at ON history(created_at DESC);
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    ).optional()
}

pub fn delete_setting(conn: &Connection, key: &str) -> Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    Ok(())
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

const HISTORY_LIMIT: i64 = 10;

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
    let id = conn.last_insert_rowid();
    // Eliminar entradas antiguas que superen el límite
    conn.execute(
        "DELETE FROM history WHERE id NOT IN (
            SELECT id FROM history ORDER BY created_at DESC LIMIT ?1
        )",
        params![HISTORY_LIMIT],
    )?;
    Ok(id)
}

pub fn get_entries(conn: &Connection, _limit: i64) -> Result<Vec<HistoryEntry>> {
    let limit = HISTORY_LIMIT;
    let mut stmt = conn.prepare(
        // Para ítems de tipo 'image', content no se necesita en la lista:
        // se usa thumbnail para la vista y se carga bajo demanda al copiar.
        // Esto evita cargar N×varios-MB en memoria de una sola vez.
        "SELECT id, type,
                CASE WHEN type = 'text' THEN content ELSE '' END,
                thumbnail, created_at
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
