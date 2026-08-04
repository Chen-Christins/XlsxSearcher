use crate::types::{Hit, PreviewResponse};
use calamine::{open_workbook_auto, Data, Reader};
use regex::bytes::Regex;
use std::fs;
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;
use zip::ZipArchive;

const SUPPORTED_EXTENSIONS: [&str; 3] = [".xlsx", ".xlsm", ".xls"];
const MAX_DEEP_INDEX_FILE_MB: u64 = 80;

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = format!(".{}", ext.to_lowercase());
            SUPPORTED_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

pub fn walk_files(directory: &str) -> Vec<(String, String, f64)> {
    let mut files = Vec::new();
    for entry in WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !entry.file_type().is_dir() || !name.starts_with('.')
        })
    {
        if let Ok(entry) = entry {
            if entry.file_type().is_file() && is_supported(entry.path()) {
                if let Ok(metadata) = fs::metadata(entry.path()) {
                    if let Ok(duration) = metadata.modified().and_then(|time| {
                        time.duration_since(std::time::UNIX_EPOCH)
                            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                    }) {
                        let mtime = duration.as_secs_f64();
                        let filepath = entry.path().to_string_lossy().to_string();
                        let filename = entry.file_name().to_string_lossy().to_string();
                        files.push((filepath, filename, mtime));
                    }
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

pub fn sheet_names(filepath: &str) -> Vec<String> {
    let path = Path::new(filepath);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if ext == "xls" {
        return sheet_names_calamine(filepath);
    }

    if let Ok(names) = sheet_names_fast_zip(filepath) {
        if !names.is_empty() {
            return names;
        }
    }
    sheet_names_calamine(filepath)
}

fn sheet_names_fast_zip(filepath: &str) -> Result<Vec<String>, String> {
    let file = fs::File::open(filepath).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    archive
        .by_name("xl/workbook.xml")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    let regex = Regex::new(r#"<sheet\s[^>]*?name="([^"]*)"#).map_err(|e| e.to_string())?;
    let names = regex
        .captures_iter(&raw)
        .filter_map(|capture| capture.get(1))
        .filter_map(|m| String::from_utf8(m.as_bytes().to_vec()).ok())
        .collect();
    Ok(names)
}

fn sheet_names_calamine(filepath: &str) -> Vec<String> {
    match open_workbook_auto(filepath) {
        Ok(workbook) => workbook.sheet_names().to_vec(),
        Err(_) => Vec::new(),
    }
}

fn data_to_string(data: &Data) -> String {
    match data {
        Data::String(value) => value.clone(),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        _ => String::new(),
    }
}

fn trim_row(row: &[String]) -> Vec<String> {
    let mut last_non_empty = None;
    for (index, value) in row.iter().enumerate() {
        if !value.is_empty() {
            last_non_empty = Some(index);
        }
    }
    match last_non_empty {
        Some(index) => row[..=index].to_vec(),
        None => Vec::new(),
    }
}

fn cell_matches(value: &str, keyword: &str, mode: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }
    let haystack = value.to_lowercase();
    let needle = keyword.to_lowercase();
    match mode {
        "exact" => haystack == needle,
        "prefix" => haystack.starts_with(&needle),
        _ => haystack.contains(&needle),
    }
}

fn worksheet_rows(filepath: &str, sheet_name: &str) -> Result<Vec<Vec<String>>, String> {
    let mut workbook = open_workbook_auto(filepath).map_err(|e| e.to_string())?;
    let range = workbook
        .worksheet_range(sheet_name)
        .map_err(|e| format!("sheet not found: {}", e))?;
    Ok(range
        .rows()
        .map(|row| row.iter().map(data_to_string).collect())
        .collect())
}

pub fn extract_cell_texts(
    filepath: &str,
    sheet_names: &[String],
    max_chars_per_sheet: usize,
) -> Result<Vec<String>, String> {
    let metadata = fs::metadata(filepath).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_DEEP_INDEX_FILE_MB * 1024 * 1024 {
        return Ok(vec![String::new(); sheet_names.len()]);
    }

    let mut workbook = open_workbook_auto(filepath).map_err(|e| e.to_string())?;
    let mut results = Vec::with_capacity(sheet_names.len());
    for sheet_name in sheet_names {
        let mut parts = Vec::new();
        let mut char_count = 0usize;
        if let Ok(range) = workbook.worksheet_range(sheet_name) {
            'outer: for row in range.rows() {
                for cell in row {
                    let value = data_to_string(cell);
                    if value.is_empty() {
                        continue;
                    }
                    char_count += value.chars().count() + 1;
                    parts.push(value);
                    if char_count > max_chars_per_sheet {
                        break 'outer;
                    }
                }
            }
        }
        results.push(parts.join(" "));
    }
    Ok(results)
}

#[allow(clippy::too_many_arguments)]
pub fn read_sheet_with_hits(
    filepath: &str,
    sheet_name: &str,
    keyword: Option<&str>,
    match_mode: &str,
    max_hits: usize,
    preview_rows: usize,
    preview_cols: usize,
    start_row: Option<usize>,
    start_col: Option<usize>,
) -> Result<PreviewResponse, String> {
    let rows = worksheet_rows(filepath, sheet_name)?;
    let header_row = rows
        .first()
        .map(|row| trim_row(&row[..row.len().min(preview_cols)]))
        .unwrap_or_default();

    let mut hits = Vec::new();
    if let Some(keyword) = keyword {
        'hits: for (row_index, row) in rows.iter().enumerate() {
            for (col_index, value) in row.iter().enumerate() {
                if !value.is_empty() && cell_matches(value, keyword, match_mode) {
                    hits.push(Hit {
                        row: row_index as i64 + 1,
                        col: col_index as i64 + 1,
                        value: value.clone(),
                    });
                    if hits.len() >= max_hits {
                        break 'hits;
                    }
                }
            }
        }
    }

    let (window_row, window_col) = if let Some(row) = start_row {
        (row, start_col.unwrap_or(1))
    } else if let Some(first_hit) = hits.first() {
        (
            (first_hit.row as usize).saturating_sub(3).max(1),
            (first_hit.col as usize).saturating_sub(2).max(1),
        )
    } else {
        (2, 1)
    };

    let data = rows
        .iter()
        .skip(window_row.saturating_sub(1))
        .take(preview_rows)
        .map(|row| {
            let start = window_col.saturating_sub(1);
            let end = (start + preview_cols).min(row.len());
            trim_row(&row[start..end])
        })
        .collect();

    Ok(PreviewResponse {
        hits,
        data,
        header_row,
        start_row: window_row,
        start_col: window_col,
    })
}
