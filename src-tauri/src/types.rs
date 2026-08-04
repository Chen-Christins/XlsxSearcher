use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
pub struct IndexStatus {
    pub file_count: i64,
    pub sheet_count: i64,
    pub indexed_cell_sheet_count: i64,
    pub pending_deep_index_count: i64,
}

#[derive(Clone, Serialize)]
pub struct AliasStats {
    pub alias_count: i64,
    pub mapping_count: i64,
}

#[derive(Clone, Serialize)]
pub struct JobState {
    pub running: bool,
    pub kind: String,
    pub current: usize,
    pub total: usize,
    pub message: String,
    pub result: Option<Value>,
    pub error: Option<String>,
}

impl Default for JobState {
    fn default() -> Self {
        Self {
            running: false,
            kind: String::new(),
            current: 0,
            total: 0,
            message: String::new(),
            result: None,
            error: None,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct SearchResult {
    pub filename: String,
    pub filepath: String,
    pub sheet_names: Vec<String>,
    pub sheet_count: usize,
    pub sheet_names_display: String,
    pub sheet_aliases: HashMap<String, Vec<String>>,
}

#[derive(Clone, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total_files: usize,
    pub total_sheets: usize,
}

#[derive(Clone, Serialize)]
pub struct Hit {
    pub row: i64,
    pub col: i64,
    pub value: String,
}

#[derive(Clone, Serialize)]
pub struct PreviewResponse {
    pub hits: Vec<Hit>,
    pub data: Vec<Vec<String>>,
    pub header_row: Vec<String>,
    pub start_row: usize,
    pub start_col: usize,
}

#[derive(Clone, Serialize)]
pub struct AppStateResponse {
    pub version: String,
    pub directory: String,
    pub index: IndexStatus,
    pub aliases: AliasStats,
    pub job: JobState,
}
