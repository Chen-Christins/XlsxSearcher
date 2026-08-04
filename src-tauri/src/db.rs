use crate::types::{
    AliasStats, IndexStatus, SearchResponse, SearchResult,
};
use indexmap::IndexMap;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection};
use std::collections::HashMap;

pub type DbPool = Pool<SqliteConnectionManager>;

const FTS_MIN_TOKEN_LEN: usize = 3;

pub fn init_db(pool: &DbPool) -> bool {
    let mut conn = pool.get().expect("get db connection");
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS xlsx_files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            filename TEXT NOT NULL,
            filepath TEXT UNIQUE NOT NULL,
            modified_time REAL NOT NULL,
            sheet_count INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS sheets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_id INTEGER NOT NULL,
            sheet_name TEXT NOT NULL,
            FOREIGN KEY (file_id) REFERENCES xlsx_files(id) ON DELETE CASCADE
        );
        CREATE TABLE IF NOT EXISTS sheet_aliases (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alias_name TEXT NOT NULL,
            sheet_name TEXT NOT NULL,
            source_path TEXT NOT NULL,
            UNIQUE(alias_name, sheet_name, source_path)
        );
        CREATE INDEX IF NOT EXISTS idx_filename ON xlsx_files(filename);
        CREATE INDEX IF NOT EXISTS idx_sheet_name ON sheets(sheet_name);
        CREATE INDEX IF NOT EXISTS idx_file_id ON sheets(file_id);
        CREATE INDEX IF NOT EXISTS idx_alias_name ON sheet_aliases(alias_name);
        CREATE INDEX IF NOT EXISTS idx_alias_sheet_name ON sheet_aliases(sheet_name);
        "#,
    )
    .expect("init sqlite schema");

    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(sheets)")
        .expect("prepare table info")
        .query_map([], |row| row.get(1))
        .expect("query table info")
        .collect::<Result<_, _>>()
        .expect("read table info");
    if !columns.iter().any(|name| name == "cell_text") {
        conn.execute_batch("ALTER TABLE sheets ADD COLUMN cell_text TEXT;")
            .expect("add cell_text column");
    }

    let fts_ok = init_fts(&mut conn);
    fts_ok
}

fn init_fts(conn: &mut Connection) -> bool {
    let result = conn.execute_batch(
        r#"
        CREATE VIRTUAL TABLE IF NOT EXISTS sheets_fts USING fts5(
            cell_text, content='sheets', content_rowid='id', tokenize='trigram'
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS sheets_fts_names USING fts5(
            sheet_name, content='sheets', content_rowid='id', tokenize='trigram'
        );
        CREATE TRIGGER IF NOT EXISTS sheets_fts_ai AFTER INSERT ON sheets BEGIN
            INSERT INTO sheets_fts(rowid, cell_text) VALUES (new.id, new.cell_text);
        END;
        CREATE TRIGGER IF NOT EXISTS sheets_fts_ad AFTER DELETE ON sheets BEGIN
            INSERT INTO sheets_fts(sheets_fts, rowid, cell_text) VALUES('delete', old.id, old.cell_text);
        END;
        CREATE TRIGGER IF NOT EXISTS sheets_fts_au AFTER UPDATE ON sheets BEGIN
            INSERT INTO sheets_fts(sheets_fts, rowid, cell_text) VALUES('delete', old.id, old.cell_text);
            INSERT INTO sheets_fts(rowid, cell_text) VALUES (new.id, new.cell_text);
        END;
        CREATE TRIGGER IF NOT EXISTS sheets_fts_names_ai AFTER INSERT ON sheets BEGIN
            INSERT INTO sheets_fts_names(rowid, sheet_name) VALUES (new.id, new.sheet_name);
        END;
        CREATE TRIGGER IF NOT EXISTS sheets_fts_names_ad AFTER DELETE ON sheets BEGIN
            INSERT INTO sheets_fts_names(sheets_fts_names, rowid, sheet_name) VALUES('delete', old.id, old.sheet_name);
        END;
        CREATE TRIGGER IF NOT EXISTS sheets_fts_names_au AFTER UPDATE ON sheets BEGIN
            INSERT INTO sheets_fts_names(sheets_fts_names, rowid, sheet_name) VALUES('delete', old.id, old.sheet_name);
            INSERT INTO sheets_fts_names(rowid, sheet_name) VALUES (new.id, new.sheet_name);
        END;
        INSERT INTO sheets_fts(rowid, cell_text) SELECT id, cell_text FROM sheets
            WHERE id NOT IN (SELECT rowid FROM sheets_fts);
        INSERT INTO sheets_fts_names(rowid, sheet_name) SELECT id, sheet_name FROM sheets
            WHERE id NOT IN (SELECT rowid FROM sheets_fts_names);
        "#,
    );
    if result.is_ok() {
        // Rebuild the external-content FTS indexes only when migrating a
        // database created by the older Python backend (which may contain stale
        // "ghost" FTS rows). Fresh or already-migrated databases stay in sync
        // via triggers, so we skip the (potentially expensive) rebuild.
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if user_version < 2 {
            let _ = conn.execute("INSERT INTO sheets_fts(sheets_fts) VALUES('rebuild')", []);
            let _ = conn.execute(
                "INSERT INTO sheets_fts_names(sheets_fts_names) VALUES('rebuild')",
                [],
            );
            let _ = conn.execute("PRAGMA user_version = 2", []);
        }
        return true;
    }

    let _ = conn.execute_batch(
        r#"
        DROP TRIGGER IF EXISTS sheets_fts_ai;
        DROP TRIGGER IF EXISTS sheets_fts_ad;
        DROP TRIGGER IF EXISTS sheets_fts_au;
        DROP TRIGGER IF EXISTS sheets_fts_names_ai;
        DROP TRIGGER IF EXISTS sheets_fts_names_ad;
        DROP TRIGGER IF EXISTS sheets_fts_names_au;
        DROP TABLE IF EXISTS sheets_fts;
        DROP TABLE IF EXISTS sheets_fts_names;
        "#,
    );
    false
}

pub fn get_indexed_snapshot(
    pool: &DbPool,
) -> Result<HashMap<String, (i64, f64)>, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, filepath, modified_time FROM xlsx_files")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, (row.get::<_, i64>(0)?, row.get::<_, f64>(2)?)))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn upsert_files_batch(
    pool: &DbPool,
    updates: &[(String, String, f64, Vec<String>)],
    indexed: &HashMap<String, (i64, f64)>,
) -> Result<(usize, usize), String> {
    let mut conn = pool.get().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let mut added = 0usize;
    let mut updated = 0usize;

    let update_file_ids: Vec<i64> = updates
        .iter()
        .filter_map(|(_, filepath, _, _)| indexed.get(filepath).map(|(id, _)| *id))
        .collect();

    let mut old_cell_texts: HashMap<(i64, String), Option<String>> = HashMap::new();
    let mut existing_sheet_names: HashMap<i64, Vec<String>> = HashMap::new();
    if !update_file_ids.is_empty() {
        let placeholders = vec!["?"; update_file_ids.len()].join(",");
        let sql = format!(
            "SELECT file_id, sheet_name, cell_text FROM sheets WHERE file_id IN ({}) ORDER BY file_id, id",
            placeholders
        );
        let mut stmt = tx.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_from_iter(update_file_ids.iter().map(|v| Value::Integer(*v))), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (file_id, sheet_name, cell_text) = row.map_err(|e| e.to_string())?;
            old_cell_texts.insert((file_id, sheet_name.clone()), cell_text);
            existing_sheet_names.entry(file_id).or_default().push(sheet_name);
        }
    }

    for (filename, filepath, mtime, sheet_names) in updates {
        if let Some((file_id, _)) = indexed.get(filepath) {
            let file_id = *file_id;
            tx.execute(
                "UPDATE xlsx_files SET filename=?, modified_time=?, sheet_count=? WHERE id=?",
                params![filename, mtime, sheet_names.len() as i64, file_id],
            )
            .map_err(|e| e.to_string())?;

            if existing_sheet_names.get(&file_id) == Some(sheet_names) {
                updated += 1;
                continue;
            }
            tx.execute("DELETE FROM sheets WHERE file_id=?", params![file_id])
                .map_err(|e| e.to_string())?;
            updated += 1;
        } else {
            tx.execute(
                "INSERT INTO xlsx_files (filename, filepath, modified_time, sheet_count) VALUES (?, ?, ?, ?)",
                params![filename, filepath, mtime, sheet_names.len() as i64],
            )
            .map_err(|e| e.to_string())?;
            let file_id = tx.last_insert_rowid();
            added += 1;
            for sheet_name in sheet_names {
                let preserved = old_cell_texts.get(&(file_id, sheet_name.clone())).cloned().flatten();
                tx.execute(
                    "INSERT INTO sheets (file_id, sheet_name, cell_text) VALUES (?, ?, ?)",
                    params![file_id, sheet_name, preserved],
                )
                .map_err(|e| e.to_string())?;
            }
            continue;
        }

        let file_id = indexed[filepath].0;
        for sheet_name in sheet_names {
            let preserved = old_cell_texts.get(&(file_id, sheet_name.clone())).cloned().flatten();
            tx.execute(
                "INSERT INTO sheets (file_id, sheet_name, cell_text) VALUES (?, ?, ?)",
                params![file_id, sheet_name, preserved],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok((added, updated))
}

pub fn delete_files_by_ids(pool: &DbPool, file_ids: &[i64]) -> Result<usize, String> {
    if file_ids.is_empty() {
        return Ok(0);
    }
    let mut conn = pool.get().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let placeholders = vec!["?"; file_ids.len()].join(",");
    let sql = format!("DELETE FROM sheets WHERE file_id IN ({})", placeholders);
    tx.execute(&sql, params_from_iter(file_ids.iter().map(|id| Value::Integer(*id))))
        .map_err(|e| e.to_string())?;
    let sql = format!("DELETE FROM xlsx_files WHERE id IN ({})", placeholders);
    tx.execute(&sql, params_from_iter(file_ids.iter().map(|id| Value::Integer(*id))))
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(file_ids.len())
}

pub fn get_index_status(pool: &DbPool) -> Result<IndexStatus, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let file_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM xlsx_files", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let sheet_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sheets", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let pending: i64 = conn
        .query_row("SELECT COUNT(*) FROM sheets WHERE cell_text IS NULL", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(IndexStatus {
        file_count,
        sheet_count,
        indexed_cell_sheet_count: (sheet_count - pending).max(0),
        pending_deep_index_count: pending,
    })
}

pub fn get_alias_stats(pool: &DbPool) -> Result<AliasStats, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let alias_count: i64 = conn
        .query_row("SELECT COUNT(DISTINCT alias_name) FROM sheet_aliases", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    let mapping_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sheet_aliases", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(AliasStats {
        alias_count,
        mapping_count,
    })
}

pub fn get_all_sheet_aliases(pool: &DbPool) -> Result<HashMap<String, Vec<String>>, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT DISTINCT alias_name, sheet_name FROM sheet_aliases ORDER BY alias_name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut result: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (alias, sheet) = row.map_err(|e| e.to_string())?;
        result.entry(sheet).or_default().push(alias);
    }
    Ok(result)
}

fn match_clause(field: &str, keyword: &str, mode: &str) -> (String, Value) {
    match mode {
        "exact" => (
            format!("LOWER({}) = LOWER(?)", field),
            Value::Text(keyword.to_string()),
        ),
        "prefix" => (
            format!("{} LIKE ? COLLATE NOCASE", field),
            Value::Text(format!("{}%", keyword)),
        ),
        _ => (
            format!("{} LIKE ? COLLATE NOCASE", field),
            Value::Text(format!("%{}%", keyword)),
        ),
    }
}

fn fts_phrase(keyword: &str) -> String {
    format!("\"{}\"", keyword.replace('"', "\"\""))
}

fn cell_condition(keyword: &str, mode: &str, fts_ok: bool) -> (String, Vec<Value>) {
    if keyword.chars().count() >= FTS_MIN_TOKEN_LEN && fts_ok {
        let mut cond = "s.id IN (SELECT rowid FROM sheets_fts WHERE sheets_fts MATCH ?)".to_string();
        let mut params = vec![Value::Text(fts_phrase(keyword))];
        if mode == "prefix" {
            cond.push_str(" AND s.cell_text LIKE ? COLLATE NOCASE");
            params.push(Value::Text(format!("{}%", keyword)));
        } else if mode == "exact" {
            cond.push_str(" AND LOWER(s.cell_text) = LOWER(?)");
            params.push(Value::Text(keyword.to_string()));
        }
        (cond, params)
    } else {
        let (clause, value) = match_clause("s.cell_text", keyword, mode);
        (clause, vec![value])
    }
}

fn sheet_condition(keywords: &[String], mode: &str, fts_ok: bool) -> (String, Vec<Value>) {
    let all_long = keywords.iter().all(|kw| kw.chars().count() >= FTS_MIN_TOKEN_LEN);
    if all_long && fts_ok {
        let terms = keywords.iter().map(|kw| fts_phrase(kw)).collect::<Vec<_>>().join(" OR ");
        let mut cond = "s.id IN (SELECT rowid FROM sheets_fts_names WHERE sheets_fts_names MATCH ?)".to_string();
        let mut params = vec![Value::Text(terms)];
        if mode == "prefix" {
            let clauses = keywords.iter().map(|_| "s.sheet_name LIKE ? COLLATE NOCASE".to_string()).collect::<Vec<_>>().join(" OR ");
            cond.push_str(&format!(" AND ({})", clauses));
            params.extend(keywords.iter().map(|kw| Value::Text(format!("{}%", kw))));
        } else if mode == "exact" {
            let clauses = keywords.iter().map(|_| "LOWER(s.sheet_name) = LOWER(?)".to_string()).collect::<Vec<_>>().join(" OR ");
            cond.push_str(&format!(" AND ({})", clauses));
            params.extend(keywords.iter().map(|kw| Value::Text(kw.clone())));
        }
        (cond, params)
    } else {
        let mut seen = std::collections::HashSet::new();
        let mut clauses = Vec::new();
        let mut params = Vec::new();
        for kw in keywords {
            let key = kw.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            let (clause, value) = match_clause("s.sheet_name", kw, mode);
            clauses.push(clause);
            params.push(value);
        }
        (format!("({})", clauses.join(" OR ")), params)
    }
}

pub fn search(
    pool: &DbPool,
    sheet_keyword: &str,
    filename_keyword: &str,
    cell_keyword: &str,
    match_mode: &str,
    sort_mode: &str,
    fts_ok: bool,
) -> Result<SearchResponse, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut sql = "SELECT DISTINCT f.filename, f.filepath, s.sheet_name FROM xlsx_files f LEFT JOIN sheets s ON f.id = s.file_id".to_string();
    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<Value> = Vec::new();

    if !filename_keyword.is_empty() {
        let (clause, value) = match_clause("f.filename", filename_keyword, match_mode);
        conditions.push(clause);
        params.push(value);
    }

    let mut sheet_keywords: Vec<String> = Vec::new();
    if !sheet_keyword.is_empty() {
        sheet_keywords.push(sheet_keyword.to_string());
        let aliases = resolve_sheet_aliases(pool, sheet_keyword, match_mode)?;
        for alias in aliases {
            if !sheet_keywords.iter().any(|kw| kw.eq_ignore_ascii_case(&alias)) {
                sheet_keywords.push(alias);
            }
        }
    }
    if !sheet_keywords.is_empty() {
        let (cond, vals) = sheet_condition(&sheet_keywords, match_mode, fts_ok);
        conditions.push(cond);
        params.extend(vals);
    }

    if !cell_keyword.is_empty() {
        let (cond, vals) = cell_condition(cell_keyword, match_mode, fts_ok);
        conditions.push(cond);
        params.extend(vals);
    }

    if !conditions.is_empty() {
        sql.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
    }
    if sort_mode == "filename_desc" {
        sql.push_str(" ORDER BY LOWER(f.filename) DESC, LOWER(s.sheet_name) DESC");
    } else {
        sql.push_str(" ORDER BY LOWER(f.filename) ASC, LOWER(s.sheet_name) ASC");
    }

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?))
        })
        .map_err(|e| e.to_string())?;

    let mut grouped: IndexMap<String, SearchResult> = IndexMap::new();
    for row in rows {
        let (filename, filepath, sheet_name) = row.map_err(|e| e.to_string())?;
        let entry = grouped.entry(filepath.clone()).or_insert_with(|| SearchResult {
            filename: filename.clone(),
            filepath: filepath.clone(),
            sheet_names: Vec::new(),
            sheet_count: 0,
            sheet_names_display: String::new(),
            sheet_aliases: HashMap::new(),
        });
        if let Some(sheet_name) = sheet_name {
            entry.sheet_names.push(sheet_name);
        }
    }

    let aliases = get_all_sheet_aliases(pool)?;
    for result in grouped.values_mut() {
        result.sheet_count = result.sheet_names.len();
        result.sheet_names_display = result.sheet_names.join(", ");
        result.sheet_aliases = result
            .sheet_names
            .iter()
            .map(|name| (name.clone(), aliases.get(name).cloned().unwrap_or_default()))
            .collect();
    }

    let results: Vec<SearchResult> = grouped.into_values().collect();
    let total_sheets = results.iter().map(|r| r.sheet_count).sum();
    Ok(SearchResponse {
        total_files: results.len(),
        total_sheets,
        results,
    })
}

pub fn resolve_sheet_aliases(
    pool: &DbPool,
    keyword: &str,
    match_mode: &str,
) -> Result<Vec<String>, String> {
    if keyword.is_empty() {
        return Ok(Vec::new());
    }
    let conn = pool.get().map_err(|e| e.to_string())?;
    let (clause, value) = match_clause("alias_name", keyword, match_mode);
    let sql = format!(
        "SELECT DISTINCT sheet_name FROM sheet_aliases WHERE {} ORDER BY LOWER(sheet_name)",
        clause
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([value], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn replace_sheet_aliases(
    pool: &DbPool,
    source_path: &str,
    mappings: &[(String, String)],
) -> Result<usize, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM sheet_aliases WHERE source_path=?", params![source_path])
        .map_err(|e| e.to_string())?;
    for (alias, sheet) in mappings {
        conn.execute(
            "INSERT OR IGNORE INTO sheet_aliases (alias_name, sheet_name, source_path) VALUES (?, ?, ?)",
            params![alias, sheet, source_path],
        )
        .map_err(|e| e.to_string())?;
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sheet_aliases WHERE source_path=?",
            params![source_path],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(count as usize)
}

pub fn get_sheets_without_cell_text(pool: &DbPool) -> Result<Vec<(i64, String, String)>, String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.sheet_name, f.filepath FROM sheets s JOIN xlsx_files f ON s.file_id=f.id WHERE s.cell_text IS NULL",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn update_sheet_cell_texts_batch(
    pool: &DbPool,
    updates: &[(String, i64)],
) -> Result<(), String> {
    let mut conn = pool.get().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (text, id) in updates {
        tx.execute(
            "UPDATE sheets SET cell_text=? WHERE id=?",
            params![text, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

pub fn clear_index(pool: &DbPool) -> Result<(), String> {
    let conn = pool.get().map_err(|e| e.to_string())?;
    conn.execute_batch("DELETE FROM sheets; DELETE FROM xlsx_files;")
        .map_err(|e| e.to_string())
}
