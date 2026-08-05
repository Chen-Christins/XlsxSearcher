use crate::alias;
use crate::app_state::AppState;
use crate::db;
use crate::scanner;
use axum::extract::{Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
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

// Custom user agent set on the desktop webview so the API can tell the built-in
// window apart from an external browser. Used to enforce enable_web_interface.
// Keeps an OS token (e.g. "macos") so the frontend can still detect the host
// platform via navigator.userAgent for the per-OS titlebar layout.
pub const WEBVIEW_UA_PREFIX: &str = "XlsxSearcher-webview";

pub fn webview_user_agent() -> String {
    format!(
        "{}/{} ({} {})",
        WEBVIEW_UA_PREFIX,
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

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
    #[serde(default)]
    pub sheet_name: Option<String>,
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
        .layer(middleware::from_fn_with_state(state.clone(), require_token))
        .layer(CorsLayer::permissive())
        .with_state(state);
    api
}

async fn require_token(
    State(state): State<SharedState>,
    request: Request,
    next: Next,
) -> Response {
    if request.uri().path() == "/api/health" {
        return next.run(request).await;
    }
    if !state.settings.lock().expect("settings lock").enable_web_interface
        && !request
            .headers()
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ua| ua.starts_with(WEBVIEW_UA_PREFIX))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "ok": false, "error": "Web 界面已关闭" })),
        )
            .into_response();
    }
    let query_token = request.uri().query().and_then(|q| {
        q.split('&')
            .find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == "token").map(|(_, v)| v.to_string()))
    });
    let cookie_token = request
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .find_map(|part| part.trim().strip_prefix("xlsx_token=").map(|v| v.to_string()))
        });
    let valid = query_token.as_deref() == Some(state.web_token.as_str())
        || cookie_token.as_deref() == Some(state.web_token.as_str());
    if !valid {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "ok": false, "error": "无权访问" })),
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    if query_token.is_some() {
        if let Ok(value) = HeaderValue::from_str(&format!("xlsx_token={}; Path=/", state.web_token)) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
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
        "open" => {
            let sheet = body.sheet_name.filter(|s| !s.is_empty());
            match sheet {
                Some(sheet) => open_file_at_sheet(&body.filepath, &sheet)
                    .map_err(ApiError::internal),
                None => open::that(&body.filepath)
                    .map_err(|e| ApiError::internal(e.to_string())),
            }?;
        }
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

// Opens a workbook and activates the given worksheet in Microsoft Excel.
// Falls back to the plain "open with default app" when Excel scripting is not
// available (e.g. Excel not installed, or a non-macOS/non-Windows platform).
fn open_file_at_sheet(filepath: &str, sheet_name: &str) -> Result<(), String> {
    let result = open_file_at_sheet_excel(filepath, sheet_name);
    if result.is_ok() {
        return result;
    }
    open::that(filepath).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn open_file_at_sheet_excel(filepath: &str, sheet_name: &str) -> Result<(), String> {
    let script = format!(
        "tell application \"Microsoft Excel\"\n\
         \topen POSIX file \"{}\"\n\
         \tactivate\n\
         \tset active sheet to worksheet \"{}\" of workbook 1\n\
         end tell",
        escape_applescript(filepath),
        escape_applescript(sheet_name),
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Excel 未能定位到指定子表".to_string())
    }
}

#[cfg(target_os = "windows")]
fn open_file_at_sheet_excel(filepath: &str, sheet_name: &str) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference='Stop'\n\
         $excel = New-Object -ComObject Excel.Application\n\
         $excel.Visible = $true\n\
         $wb = $excel.Workbooks.Open('{}')\n\
         $ws = $wb.Worksheets.Item('{}')\n\
         $ws.Activate()\n\
         $excel.Activate()\n",
        filepath.replace('\'', "''"),
        sheet_name.replace('\'', "''"),
    );
    let status = std::process::Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("Excel 未能定位到指定子表".to_string())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn open_file_at_sheet_excel(_filepath: &str, _sheet_name: &str) -> Result<(), String> {
    Err("当前平台不支持定位子表".to_string())
}

fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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

async fn open_browser_handler(
    State(state): State<SharedState>,
    Json(body): Json<OpenBrowserBody>,
) -> ApiResult {
    if !state.settings.lock().expect("settings lock").enable_web_interface {
        return Err(ApiError::bad_request("Web 界面已关闭"));
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Mutex;
    use tower::ServiceExt;

    async fn router_with_web_interface(enabled: bool) -> Router {
        let state = crate::app_state::AppState::new();
        state.settings.lock().unwrap().enable_web_interface = enabled;
        build_router(state)
    }

    #[tokio::test]
    async fn web_interface_off_blocks_external_browser() {
        let router = router_with_web_interface(false).await;
        let res = router
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .header(header::USER_AGENT, "Mozilla/5.0 (Macintosh)")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn web_interface_off_allows_webview_with_token() {
        let state = crate::app_state::AppState::new();
        state.settings.lock().unwrap().enable_web_interface = false;
        let token = state.web_token.clone();
        let router = build_router(state);
        let res = router
            .oneshot(
                Request::builder()
                    .uri(&format!("/api/state?token={}", token))
                    .header(header::USER_AGENT, WEBVIEW_UA_PREFIX)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn web_interface_off_blocks_webview_without_token() {
        let router = router_with_web_interface(false).await;
        let res = router
            .oneshot(
                Request::builder()
                    .uri("/api/state")
                    .header(header::USER_AGENT, WEBVIEW_UA_PREFIX)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn stale_cookie_is_refreshed_so_assets_still_load() {
        let state = crate::app_state::AppState::new();
        state.settings.lock().unwrap().enable_web_interface = true;
        let router = build_router(state.clone());
        let token = state.web_token.clone();

        // Page load with a valid query token but a stale cookie from a previous
        // launch (the fixed port persists cookies across sessions).
        let res = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&format!("/?token={}", token))
                    .header(header::USER_AGENT, "Mozilla/5.0")
                    .header(header::COOKIE, "xlsx_token=stale-old-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let set_cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(set_cookie.contains("xlsx_token="), "cookie should be refreshed");

        // Asset request carrying the refreshed cookie must load.
        let res = router
            .oneshot(
                Request::builder()
                    .uri("/styles.css")
                    .header(header::USER_AGENT, "Mozilla/5.0")
                    .header(header::COOKIE, &set_cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(res.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap().starts_with("text/css"));
    }

    // --- deep index regression tests ---------------------------------------

    fn make_test_workbook(path: &std::path::Path) {
        use std::io::Write;
        let file = std::fs::File::create(path).expect("create workbook");
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#,
        )
        .unwrap();
        zip.start_file("_rels/.rels", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#,
        )
        .unwrap();
        zip.start_file("xl/workbook.xml", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets>
<sheet name="SheetA" sheetId="1" r:id="rId1"/>
<sheet name="SheetB" sheetId="2" r:id="rId2"/>
</sheets>
</workbook>"#,
        )
        .unwrap();
        zip.start_file("xl/_rels/workbook.xml.rels", opts).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#,
        )
        .unwrap();
        for (idx, rows) in [
            vec![
                vec![("A1", "apple"), ("B1", "banana"), ("C1", "orange")],
                vec![("A2", "zhang"), ("B2", "100"), ("C2", "beijing")],
            ],
            vec![
                vec![("A1", "header"), ("B1", "value")],
                vec![("A2", "hello"), ("B2", "world")],
            ],
        ]
        .iter()
        .enumerate()
        {
            zip.start_file(format!("xl/worksheets/sheet{}.xml", idx + 1), opts).unwrap();
            let mut xml = String::from(
                r#"<?xml version="1.0"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
            );
            for (row, cells) in rows.iter().enumerate() {
                xml.push_str(&format!(r#"<row r="{}">"#, row + 1));
                for (ref_, value) in cells {
                    xml.push_str(&format!(
                        r#"<c r="{}" t="inlineStr"><is><t>{}</t></is></c>"#,
                        ref_, value
                    ));
                }
                xml.push_str("</row>");
            }
            xml.push_str("</sheetData></worksheet>");
            zip.write_all(xml.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn make_temp_state(dir: &std::path::Path) -> Arc<AppState> {
        let manager = r2d2_sqlite::SqliteConnectionManager::file(dir.join("index.db")).with_init(
            |conn| {
                conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0))?;
                conn.execute("PRAGMA synchronous=NORMAL", [])?;
                conn.execute("PRAGMA foreign_keys=ON", [])?;
                Ok(())
            },
        );
        let pool = r2d2::Pool::builder()
            .max_size(4)
            .min_idle(Some(0))
            .build(manager)
            .expect("create test pool");
        let fts_available = crate::db::init_db(&pool);
        Arc::new(crate::app_state::AppState {
            pool,
            fts_available,
            version: "test".to_string(),
            web_token: "test-token".to_string(),
            directory: Mutex::new(String::new()),
            settings: Mutex::new(crate::types::Settings::default()),
            job: Mutex::new(crate::types::JobState::default()),
            app_handle: Mutex::new(None),
            state_file: dir.join("webui_state.json"),
        })
    }

    #[test]
    fn deep_index_runs_twice_without_hanging() {
        let dir = std::env::temp_dir().join(format!("xlsxsearcher-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        make_test_workbook(&dir.join("dataset.xlsx"));
        let state = make_temp_state(&dir);
        let scan_dir = dir.to_string_lossy().to_string();

        // First scan: indexes the file and auto deep-indexes both sheets.
        scan_worker(state.clone(), scan_dir.clone());
        assert!(
            !state.job.lock().unwrap().running,
            "job should be finished after first scan: {:?}",
            state.job.lock().unwrap().error
        );
        let pending = crate::db::get_index_status(&state.pool).unwrap().pending_deep_index_count;
        assert_eq!(pending, 0, "first scan should deep-index all sheets");

        // Second scan of the same files must not hang and must be a no-op.
        let start = std::time::Instant::now();
        scan_worker(state.clone(), scan_dir.clone());
        assert!(!state.job.lock().unwrap().running, "job stuck after second scan");
        assert!(start.elapsed().as_secs() < 5, "second scan too slow");

        // Manual deep index on an up-to-date index must complete immediately.
        let result = deep_index_internal(state.clone()).unwrap();
        assert_eq!(result, (0, 0), "second deep index should be a no-op");

        std::fs::remove_dir_all(&dir).ok();
    }
}
