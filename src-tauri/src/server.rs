use crate::alias;
use crate::app_state::AppState;
use crate::db;
use crate::scanner;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use include_dir::{include_dir, Dir};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

static WEBUI_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../webui/dist");

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

async fn webui_fallback_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = if path.is_empty() || path == "index.html" {
        WEBUI_DIST.get_file("index.html")
    } else {
        WEBUI_DIST
            .get_file(path)
            .or_else(|| WEBUI_DIST.get_file("index.html"))
    };
    match file {
        Some(file) => (
            [
                (
                    header::CONTENT_TYPE,
                    content_type(file.path().to_str().unwrap_or("")),
                )
            ],
            file.contents(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}

pub type SharedState = Arc<AppState>;

pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "ok": false, "error": self.message })),
        )
            .into_response()
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub sheet: Option<String>,
    pub filename: Option<String>,
    pub cell: Option<String>,
    pub match_mode: Option<String>,
    pub sort_mode: Option<String>,
}

#[derive(Deserialize)]
pub struct PreviewQuery {
    pub filepath: String,
    pub sheet_name: String,
    pub keyword: Option<String>,
    pub match_mode: Option<String>,
    pub start_row: Option<usize>,
    pub start_col: Option<usize>,
}

#[derive(Deserialize)]
pub struct ScanBody {
    pub directory: String,
}

#[derive(Deserialize)]
pub struct ActionBody {
    pub action: String,
    pub filepath: String,
}

#[derive(Deserialize)]
pub struct ImportAliasesBody {
    pub source_name: Option<String>,
    pub content: String,
}

#[derive(Deserialize)]
pub struct ImportAliasesFileBody {
    pub path: String,
}

#[derive(Deserialize)]
pub struct ExportBody {
    pub path: String,
    pub sheet: Option<String>,
    pub filename: Option<String>,
    pub cell: Option<String>,
    pub match_mode: Option<String>,
    pub sort_mode: Option<String>,
}

#[derive(Deserialize)]
pub struct WindowActionBody {
    pub action: String,
}

#[derive(Deserialize)]
pub struct OpenBrowserBody {
    pub url: String,
}

pub fn build_router(state: SharedState) -> Router {
    let api = Router::new()
        .route("/api/state", get(state_handler))
        .route("/api/search", get(search_handler))
        .route("/api/preview", get(preview_handler))
        .route("/api/health", get(health_handler))
        .route("/api/scan", post(scan_handler))
        .route("/api/deep-index", post(deep_index_handler))
        .route("/api/clear-index", post(clear_index_handler))
        .route("/api/action", post(action_handler))
        .route("/api/import-aliases", post(import_aliases_handler))
        .route("/api/import-aliases-file", post(import_aliases_file_handler))
        .route("/api/export", post(export_handler))
        .route("/api/choose-directory", post(choose_directory_handler))
        .route("/api/choose-alias-file", post(choose_alias_file_handler))
        .route("/api/choose-export-file", post(choose_export_file_handler))
        .route("/api/window-action", post(window_action_handler))
        .route("/api/open-browser", post(open_browser_handler))
        .route("/api/settings", post(settings_handler))
        .fallback(webui_fallback_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);
    api
}

async fn state_handler(State(state): State<SharedState>) -> ApiResult {
    let mut value = serde_json::to_value(state.get_state()).map_err(|e| ApiError::internal(e.to_string()))?;
    value["ok"] = json!(true);
    Ok(Json(value))
}

async fn health_handler() -> ApiResult {
    Ok(Json(json!({ "ok": true })))
}

async fn search_handler(
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult {
    let response = db::search(
        &state.pool,
        query.sheet.as_deref().unwrap_or(""),
        query.filename.as_deref().unwrap_or(""),
        query.cell.as_deref().unwrap_or(""),
        query.match_mode.as_deref().unwrap_or("fuzzy"),
        query.sort_mode.as_deref().unwrap_or("filename_asc"),
        state.fts_available,
    )
    .map_err(|e| ApiError::internal(e))?;
    Ok(Json(json!({ "ok": true, "results": response.results, "total_files": response.total_files, "total_sheets": response.total_sheets })))
}

async fn preview_handler(
    State(_state): State<SharedState>,
    Query(query): Query<PreviewQuery>,
) -> ApiResult {
    if query.filepath.is_empty() || query.sheet_name.is_empty() {
        return Err(ApiError::bad_request("filepath 和 sheet_name 不能为空"));
    }
    if !std::path::Path::new(&query.filepath).exists() {
        return Err(ApiError {
            status: StatusCode::NOT_FOUND,
            message: "文件不存在或已被移动".to_string(),
        });
    }
    let response = scanner::read_sheet_with_hits(
        &query.filepath,
        &query.sheet_name,
        query.keyword.as_deref().filter(|k| !k.is_empty()),
        query.match_mode.as_deref().unwrap_or("fuzzy"),
        200,
        20,
        50,
        query.start_row,
        query.start_col,
    )
    .map_err(|e| ApiError::internal(e))?;
    Ok(Json(json!({
        "ok": true,
        "hits": response.hits,
        "data": response.data,
        "header_row": response.header_row,
        "start_row": response.start_row,
        "start_col": response.start_col,
    })))
}

async fn scan_handler(
    State(state): State<SharedState>,
    Json(body): Json<ScanBody>,
) -> ApiResult {
    let directory = std::path::Path::new(&body.directory);
    if !directory.is_dir() {
        return Err(ApiError::bad_request("目录不存在或不可读"));
    }
    {
        let mut job = state.job.lock().expect("job lock");
        if job.running {
            return Err(ApiError::conflict("已有任务正在执行"));
        }
        job.running = true;
        job.kind = "scan".to_string();
        job.current = 0;
        job.total = 0;
        job.message = "正在扫描...".to_string();
        job.result = None;
        job.error = None;
    }
    state.set_directory(body.directory.clone());

    let worker_state = state.clone();
    std::thread::spawn(move || {
        scan_worker(worker_state, body.directory);
    });
    Ok(Json(json!({ "ok": true })))
}

fn scan_worker(state: SharedState, directory: String) {
    let result = (|| -> Result<(usize, usize, usize), String> {
        let files = scanner::walk_files(&directory);
        let indexed = db::get_indexed_snapshot(&state.pool)?;
        let mut to_process: Vec<(String, String, f64)> = Vec::new();
        for (filepath, filename, mtime) in files {
            match indexed.get(&filepath) {
                Some((_, old_mtime)) if (*old_mtime - mtime).abs() < f64::EPSILON => {}
                _ => to_process.push((filepath, filename, mtime)),
            }
        }
        state.set_progress(0, to_process.len());

        let updates: Vec<(String, String, f64, Vec<String>)> = to_process
            .par_iter()
            .filter_map(|(filepath, filename, mtime)| {
                let names = scanner::sheet_names(filepath);
                if names.is_empty() {
                    None
                } else {
                    Some((filename.clone(), filepath.clone(), *mtime, names))
                }
            })
            .collect();
        state.set_progress(updates.len(), to_process.len());

        let (added, updated) = db::upsert_files_batch(&state.pool, &updates, &indexed)?;

        let files_on_disk: HashMap<String, ()> = scanner::walk_files(&directory)
            .into_iter()
            .map(|(path, _, _)| (path, ()))
            .collect();
        let ids_to_delete: Vec<i64> = indexed
            .iter()
            .filter(|(path, _)| !files_on_disk.contains_key(*path))
            .map(|(_, (id, _))| *id)
            .collect();
        let deleted = db::delete_files_by_ids(&state.pool, &ids_to_delete)?;
        Ok((added, updated, deleted))
    })();

    match result {
        Ok((added, updated, deleted)) => {
            // Auto deep-index newly added/changed sheets so cell-content search
            // works right after a scan without a manual "深度索引" step.
            let pending = db::get_index_status(&state.pool)
                .map(|s| s.pending_deep_index_count)
                .unwrap_or(0);
            if pending > 0 {
                {
                    let mut job = state.job.lock().expect("job lock");
                    job.message = "扫描完成，正在深度索引...".to_string();
                }
                match deep_index_internal(state.clone()) {
                    Ok((processed, errors)) => {
                        let mut message = format!(
                            "扫描完成：新增 {}，更新 {}，删除 {}",
                            added, updated, deleted
                        );
                        if processed > 0 {
                            message.push_str(&format!("；已深度索引 {} 个子表", processed));
                        }
                        state.finish_job(
                            Some(json!({
                                "added": added,
                                "updated": updated,
                                "deleted": deleted,
                                "deep_indexed": processed,
                                "errors": errors,
                            })),
                            message,
                            None,
                        );
                    }
                    Err(error) => state.finish_job(None, String::new(), Some(error)),
                }
            } else {
                state.finish_job(
                    Some(json!({ "added": added, "updated": updated, "deleted": deleted })),
                    format!("扫描完成：新增 {}，更新 {}，删除 {}", added, updated, deleted),
                    None,
                );
            }
        }
        Err(error) => state.finish_job(None, String::new(), Some(error)),
    }
}

async fn deep_index_handler(State(state): State<SharedState>) -> ApiResult {
    {
        let mut job = state.job.lock().expect("job lock");
        if job.running {
            return Err(ApiError::conflict("已有任务正在执行"));
        }
        job.running = true;
        job.kind = "deep_index".to_string();
        job.current = 0;
        job.total = 0;
        job.message = "正在提取单元格内容...".to_string();
        job.result = None;
        job.error = None;
    }

    let worker_state = state.clone();
    std::thread::spawn(move || {
        deep_index_worker(worker_state);
    });
    Ok(Json(json!({ "ok": true })))
}

fn deep_index_worker(state: SharedState) {
    let result = deep_index_internal(state.clone());
    match result {
        Ok((processed, errors)) => {
            let message = if processed == 0 {
                "深度索引已是最新".to_string()
            } else {
                format!("深度索引完成：已处理 {} 个子表", processed)
            };
            state.finish_job(Some(json!({ "processed": processed, "errors": errors })), message, None);
        }
        Err(error) => state.finish_job(None, String::new(), Some(error)),
    }
}

fn deep_index_internal(state: SharedState) -> Result<(usize, usize), String> {
    let pending = db::get_sheets_without_cell_text(&state.pool)?;
    let total = pending.len();
    if total == 0 {
        return Ok((0, 0));
    }
    let mut by_file: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for (sheet_id, sheet_name, filepath) in pending {
        by_file.entry(filepath).or_default().push((sheet_id, sheet_name));
    }

    let files: Vec<(String, Vec<(i64, String)>)> = by_file.into_iter().collect();
    let extracted: Vec<Result<(String, Vec<String>), String>> = files
        .par_iter()
        .map(|(filepath, entries)| {
            let names: Vec<String> = entries.iter().map(|(_, name)| name.clone()).collect();
            scanner::extract_cell_texts(filepath, &names, 50_000_000)
                .map(|texts| (filepath.clone(), texts))
        })
        .collect();

    let mut processed = 0usize;
    let mut errors = 0usize;
    let mut updates: Vec<(String, i64)> = Vec::new();
    for result in extracted {
        match result {
            Ok((filepath, texts)) => {
                let empty: Vec<(i64, String)> = Vec::new();
                let entries = files
                    .iter()
                    .find(|(path, _)| *path == filepath)
                    .map(|(_, entries)| entries)
                    .unwrap_or(&empty);
                for ((sheet_id, _), text) in entries.iter().zip(texts.iter()) {
                    updates.push((text.clone(), *sheet_id));
                }
                processed += entries.len();
            }
            Err(_) => {
                errors += 1;
            }
        }
    }
    db::update_sheet_cell_texts_batch(&state.pool, &updates)?;
    state.set_progress(processed, total);
    Ok((processed, errors))
}

async fn clear_index_handler(State(state): State<SharedState>) -> ApiResult {
    let running = state.job.lock().expect("job lock").running;
    if running {
        return Err(ApiError::conflict("任务执行中，请稍后再清空索引"));
    }
    db::clear_index(&state.pool).map_err(|e| ApiError::internal(e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn action_handler(
    State(_state): State<SharedState>,
    Json(body): Json<ActionBody>,
) -> ApiResult {
    if body.filepath.is_empty() {
        return Err(ApiError::bad_request("缺少文件路径"));
    }
    match body.action.as_str() {
        "open" => open::that(&body.filepath).map_err(|e| ApiError::internal(e.to_string()))?,
        "locate" => locate_file(&body.filepath).map_err(|e| ApiError::internal(e.to_string()))?,
        "copy" => {
            let mut clipboard = arboard::Clipboard::new().map_err(|e| ApiError::internal(e.to_string()))?;
            clipboard
                .set_text(body.filepath.clone())
                .map_err(|e| ApiError::internal(e.to_string()))?;
        }
        _ => return Err(ApiError::bad_request(format!("未知操作: {}", body.action))),
    }
    Ok(Json(json!({ "ok": true })))
}

fn locate_file(filepath: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        command.arg("-R").arg(filepath);
        command.spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", filepath))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let directory = std::path::Path::new(filepath)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        open::that(directory).map_err(|e| e.to_string())?;
        Ok(())
    }
}

async fn import_aliases_handler(
    State(state): State<SharedState>,
    Json(body): Json<ImportAliasesBody>,
) -> ApiResult {
    let mappings = alias::parse_alias_text(&body.content);
    if mappings.is_empty() {
        return Err(ApiError::bad_request("未解析到有效映射"));
    }
    let source = body.source_name.unwrap_or_else(|| "imported.txt".to_string());
    let imported = db::replace_sheet_aliases(&state.pool, &source, &mappings)
        .map_err(|e| ApiError::internal(e))?;
    Ok(Json(json!({ "ok": true, "imported": imported, "mapping_count": mappings.len() })))
}

async fn import_aliases_file_handler(
    State(state): State<SharedState>,
    Json(body): Json<ImportAliasesFileBody>,
) -> ApiResult {
    if !std::path::Path::new(&body.path).is_file() {
        return Err(ApiError::bad_request("映射文件不存在"));
    }
    let content = alias::read_alias_file(&body.path).map_err(|e| ApiError::internal(e))?;
    let mappings = alias::parse_alias_text(&content);
    if mappings.is_empty() {
        return Err(ApiError::bad_request("未解析到有效映射"));
    }
    let source = fs::canonicalize(&body.path)
        .unwrap_or_else(|_| PathBuf::from(&body.path))
        .to_string_lossy()
        .to_string();
    let imported = db::replace_sheet_aliases(&state.pool, &source, &mappings)
        .map_err(|e| ApiError::internal(e))?;
    Ok(Json(json!({ "ok": true, "imported": imported, "mapping_count": mappings.len() })))
}

async fn export_handler(
    State(state): State<SharedState>,
    Json(body): Json<ExportBody>,
) -> ApiResult {
    let response = db::search(
        &state.pool,
        body.sheet.as_deref().unwrap_or(""),
        body.filename.as_deref().unwrap_or(""),
        body.cell.as_deref().unwrap_or(""),
        body.match_mode.as_deref().unwrap_or("fuzzy"),
        body.sort_mode.as_deref().unwrap_or("filename_asc"),
        state.fts_available,
    )
    .map_err(|e| ApiError::internal(e))?;

    if let Some(parent) = std::path::Path::new(&body.path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut csv = String::from("\u{feff}文件名,子表名称,别名,文件路径,命中子表数\n");
    for result in &response.results {
        if result.sheet_names.is_empty() {
            csv.push_str(&format!(
                "\"{}\",\"\",\"\",\"{}\",{}\n",
                result.filename.replace('"', "\"\""),
                result.filepath.replace('"', "\"\""),
                result.sheet_count
            ));
        } else {
            for sheet in &result.sheet_names {
                let aliases = result.sheet_aliases.get(sheet).cloned().unwrap_or_default().join("; ");
                csv.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",{}\n",
                    result.filename.replace('"', "\"\""),
                    sheet.replace('"', "\"\""),
                    aliases.replace('"', "\"\""),
                    result.filepath.replace('"', "\"\""),
                    result.sheet_count
                ));
            }
        }
    }
    fs::write(&body.path, csv).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true, "exported": response.total_files, "path": body.path })))
}

async fn choose_directory_handler() -> ApiResult {
    match rfd::FileDialog::new().pick_folder() {
        Some(path) => Ok(Json(json!({ "ok": true, "path": path.to_string_lossy() }))),
        None => Ok(Json(json!({ "ok": true, "path": null }))),
    }
}

async fn choose_alias_file_handler() -> ApiResult {
    match rfd::FileDialog::new()
        .add_filter("Text files", &["bat", "txt", "cmd"])
        .pick_file()
    {
        Some(path) => Ok(Json(json!({ "ok": true, "path": path.to_string_lossy() }))),
        None => Ok(Json(json!({ "ok": true, "path": null }))),
    }
}

async fn choose_export_file_handler() -> ApiResult {
    match rfd::FileDialog::new()
        .add_filter("CSV files", &["csv"])
        .set_file_name("xlsx_search_results.csv")
        .save_file()
    {
        Some(path) => Ok(Json(json!({ "ok": true, "path": path.to_string_lossy() }))),
        None => Ok(Json(json!({ "ok": true, "path": null }))),
    }
}

async fn window_action_handler(
    State(state): State<SharedState>,
    Json(body): Json<WindowActionBody>,
) -> ApiResult {
    let handle = state.app_handle.lock().expect("app handle lock").clone();
    let Some(handle) = handle else {
        return Err(ApiError::bad_request("窗口尚未就绪"));
    };
    use tauri::Manager;
    let Some(window) = handle.get_webview_window("main") else {
        return Err(ApiError::bad_request("找不到主窗口"));
    };
    match body.action.as_str() {
        "close" => window.close().map_err(|e| ApiError::internal(e.to_string()))?,
        "minimize" => window.minimize().map_err(|e| ApiError::internal(e.to_string()))?,
        "maximize" => {
            if window.is_maximized().map_err(|e| ApiError::internal(e.to_string()))? {
                window.unmaximize().map_err(|e| ApiError::internal(e.to_string()))?;
            } else {
                window.maximize().map_err(|e| ApiError::internal(e.to_string()))?;
            }
        }
        _ => return Err(ApiError::bad_request(format!("未知操作: {}", body.action))),
    }
    Ok(Json(json!({ "ok": true })))
}

async fn open_browser_handler(Json(body): Json<OpenBrowserBody>) -> ApiResult {
    if body.url.is_empty() {
        return Err(ApiError::bad_request("缺少 URL"));
    }
    open::that(&body.url).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

async fn settings_handler(
    State(state): State<SharedState>,
    Json(settings): Json<crate::types::Settings>,
) -> ApiResult {
    state.set_settings(settings);
    Ok(Json(json!({ "ok": true })))
}
