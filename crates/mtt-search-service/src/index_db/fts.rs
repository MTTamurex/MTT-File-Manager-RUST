use std::path::Path;

use rusqlite::{params, Connection};

const FTS_READER_BUSY_TIMEOUT_MS: u64 = 2_000;

/// A single FTS5 search match.
pub struct FtsMatch {
    pub frn: u64,
    pub drive_letter: char,
    pub name: String,
    pub is_dir: bool,
}

/// Read-only searcher for FTS5 queries.
///
/// Opens a **separate** SQLite connection in read-only mode so FTS5 queries
/// never contend with the writer (`IndexDb`). WAL mode allows both to operate
/// concurrently.
pub struct FtsSearcher {
    db_path: std::path::PathBuf,
}

impl FtsSearcher {
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_read_connection(path)?;
        Ok(Self {
            db_path: path.to_path_buf(),
        })
    }

    fn open_read_connection(path: &Path) -> Result<Connection, String> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("FTS searcher open error: {}", e))?;
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout={};",
            FTS_READER_BUSY_TIMEOUT_MS
        ))
            .map_err(|e| format!("FTS searcher PRAGMA error: {}", e))?;
        Ok(conn)
    }

    /// Query FTS5 for file names matching `query` (substring match via trigram tokenizer).
    ///
    /// Multi-word queries require ALL words to appear as substrings (implicit AND).
    /// Returns up to `limit` results starting at `offset`.
    pub fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<FtsMatch>, String> {
        if query.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let fts_query = build_fts5_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let conn = Self::open_read_connection(&self.db_path)?;

        let mut stmt = conn
            .prepare_cached(
                "SELECT r.frn, r.drive_letter, r.name, r.is_dir
                 FROM search_fts f
                 JOIN file_records r ON r.rowid = f.rowid
                 WHERE search_fts MATCH ?1
                 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| format!("FTS search prepare error: {}", e))?;

        let rows = stmt
            .query_map(
                params![fts_query, limit as i64, offset as i64],
                |row| {
                    let frn: i64 = row.get(0)?;
                    let drive_letter: String = row.get(1)?;
                    let name: String = row.get(2)?;
                    let is_dir: bool = row.get(3)?;
                    Ok(FtsMatch {
                        frn: frn as u64,
                        drive_letter: drive_letter.chars().next().unwrap_or('C'),
                        name,
                        is_dir,
                    })
                },
            )
            .map_err(|e| format!("FTS search query error: {}", e))?;

        let mut results = Vec::with_capacity(limit.min(1024));
        for row in rows {
            match row {
                Ok(m) => results.push(m),
                Err(e) => {
                    eprintln!("[FTS] Error reading search result: {}", e);
                }
            }
        }

        Ok(results)
    }
}

/// Build an FTS5 query string for the trigram tokenizer.
///
/// Each whitespace-delimited token is quoted (implicit AND in FTS5).
/// Example: `"report" "xlsx"` matches names containing both substrings.
fn build_fts5_query(query: &str) -> String {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return String::new();
    }
    tokens
        .iter()
        .map(|t| {
            let escaped = t.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect::<Vec<_>>()
        .join(" ")
}
