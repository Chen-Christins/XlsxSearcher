# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
```bash
# Install root deps (Tauri CLI) and the web UI
npm install
cd webui && npm install && npm run build && cd ..

# Start the desktop app (Tauri, requires display)
npm run tauri -- dev
```

There is no test suite, linter config, or typecheck setup in this repo. The quickest verification is `cd webui && npm run build` (TypeScript) followed by `cargo build` in `src-tauri` (Rust).

### Packaging
```bash
# Tauri bundles (macOS dmg/app, Windows nsis, Linux deb/appimage)
npm run tauri -- build --bundles app,dmg   # or nsis / deb,appimage
```

CI (`.github/workflows/build.yml`) builds on push to `master`, PRs to `master`, version tags (`v*`), and manual dispatch. The matrix builds macOS (universal-apple-darwin, app+dmg), Windows (nsis), and Ubuntu (deb+appimage). On tag pushes it creates a draft GitHub Release and attaches per-platform `.zip` artifacts.

## Architecture

The shipped desktop client is a **Tauri 2 (Rust)** app:

```
package.json            ← root config; `npm run tauri` script, @tauri-apps/cli
src-tauri/src/main.rs   ← entrypoint: boots axum on 127.0.0.1:0, opens frameless webview
src-tauri/src/server.rs ← local HTTP API + embedded webui/dist (include_dir!)
src-tauri/src/db.rs     ← SQLite schema (FTS5 trigram) + all search queries
src-tauri/src/scanner.rs← file discovery, sheet-name extraction, cell reading (calamine)
src-tauri/src/alias.rs  ← alias-map .txt parsing (two formats, see below)
webui/src               ← TypeScript UI source, compiled to webui/dist
```

`main.rs` starts a local axum HTTP server, then opens a webview pointing at `http://127.0.0.1:<port>` (macOS: `TitleBarStyle::Overlay` with native traffic lights; elsewhere: `decorations(false)` frameless). The Rust server serves `webui/dist` (embedded at compile time via `include_dir!`) as the fallback route, so packaged builds never depend on a runtime filesystem path. Native operations are bridged over HTTP: the UI calls `/api/choose-directory`, `/api/choose-alias-file`, `/api/choose-export-file`, `/api/window-action`, `/api/open-browser`, with web fallbacks (file-input / blob download / `window.open`) for plain-browser mode.

### Threading model
- **Scan job** — spawned on a `std::thread`; walks files with `walkdir`, sheet extraction parallelized with `rayon`; SQLite writes go through the r2d2 pool (`src-tauri/src/db.rs`).
- **Deep index job** — `rayon` `par_iter` groups `cell_text IS NULL` sheets by file and extracts via `scanner::extract_cell_texts`; batch updates go through the r2d2 pool. New/changed sheets are deep-indexed automatically right after a scan (`scan_worker` calls `deep_index_internal`), so cell search works without a manual "深度索引" step; unchanged re-scans skip it.
- **Search / preview** — handled on axum request threads; the frontend owns request supersession via sequence counters.

### SQLite connections
`src-tauri/src/db.rs` uses an r2d2 pool of `rusqlite` connections (`max_size=4, min_idle=0`, lazily created) in WAL mode. Each pooled connection runs `PRAGMA journal_mode=WAL; synchronous=NORMAL; foreign_keys=ON` on creation. `sheets.cell_text` is backed by an FTS5 trigram external-content table (`sheets_fts`, plus `sheets_fts_names` for sheet names) kept in sync by AFTER INSERT/UPDATE/DELETE triggers. `init_fts` runs `rebuild` on those tables at startup so databases migrated from the old Python backend are re-indexed correctly.

### Data flow
1. User selects a directory through the native dialog (`/api/choose-directory`) or a directory-picker fallback → a scan job walks it with `walkdir`, skips hidden dirs by `.` prefix, and extracts sheet names from `xl/workbook.xml` via compiled regex (fast path) or falls back to calamine.
2. Sheet names are written to `~/.local/XlsxSearcher/index.db` (SQLite, WAL mode). The DB has three tables: `xlsx_files`, `sheets`, `sheet_aliases`.
3. "Deep index" extracts cell text from every sheet via `scanner::extract_cell_texts` (calamine), stored in `sheets.cell_text`. Files >80MB are skipped.
4. Searches query the SQLite index directly — results are grouped by file in `db::search`. Cell-content search (>=3-char keyword) uses the FTS5 trigram index (`sheets_fts MATCH`); shorter keywords fall back to `LIKE`.
5. Sheet preview reads a 20-row × 50-col window via `scanner::read_sheet_with_hits()` on an axum request thread, with window positioning around hit cells. For xlsx/xlsm it streams the sheet XML with a byte-level scanner (no full-sheet materialization): it scans for hits in one pass (stopping at `max_hits`), then extracts only the window rows in a second pass (stopping once past the window). Falls back to calamine for `.xls` or unreadable files.

### Match modes
Three modes, applied at the SQL query level for sheet-name/filename: `fuzzy` (`LIKE '%keyword%'`), `prefix` (`LIKE 'keyword%'`), `exact` (`= keyword`). All use `COLLATE NOCASE`. For cell-content search, FTS5 trigram handles the fuzzy (substring) case directly; `prefix`/`exact` FTS-narrow then apply the corresponding LIKE/`LOWER()=` post-filter to preserve semantics.

### Alias mapping
`src-tauri/src/alias.rs` supports two formats in `.txt` mapping files:
- **Standard** (recommended): `EnglishConfigName SheetName1 SheetName2 ...`
- **Legacy**: `call <script> <AliasNames...> <SheetNames...>` (Cartesian product of aliases × sheet names)

Comments are `#`, `::`, or `REM`. The parser tries encodings UTF-8-SIG → UTF-8 → GBK.

### File format support
`.xlsx`, `.xlsm` and `.xls` are supported. Cell reading uses `calamine` (Rust-level XML parsing, ~5-10x faster than openpyxl). Sheet-name extraction uses a compiled regex on `xl/workbook.xml` raw bytes (~2-3x faster than full XML parse).

## Key details

- **Index database**: `~/.local/XlsxSearcher/index.db` on all platforms. `clear_index()` wipes this user-level database (affects the app globally, not just the workspace).
- **Preview panel**: bound to `Ctrl+`` / `Cmd+`` toggle. Shows rows starting from `start_row`/`start_col` around the first hit. Hit cells are highlighted; the current hit is marked distinctly.
- **Search history**: not yet implemented in the TypeScript UI.
- **Scanner**: uses `walkdir`, skips directories whose name starts with `.`. Files are sorted by full path before processing for better disk locality.
- **Progress reporting**: job progress is polled by the frontend via `/api/state`.
- **Upsert batch writes**: `db::upsert_files_batch()` preserves existing `cell_text` across re-scans by querying old values before the DELETE+INSERT cycle. When a file's sheet-name list is unchanged since the last scan, the sheets rows are NOT rebuilt — only `xlsx_files.modified_time` is updated, so deep-indexed `cell_text` is trivially retained.
- **Combined file open**: `scanner::read_sheet_with_hits()` opens a file once and returns hits + preview data + header row simultaneously. For xlsx/xlsm it uses a streaming byte scanner (`scan_sheet`/`scan_row_cells` in `src-tauri/src/scanner.rs`) that avoids materializing the whole sheet; `.xls` and failures fall back to the calamine path.
- **Preview rendering**: `webui/src/app.ts` has `renderPreview()` that takes already-loaded data (no file I/O). The frontend calls the `/api/preview` endpoint; do NOT call `scanner::read_sheet_with_hits()` synchronously on the UI thread.
- **Window chrome**: macOS uses `TitleBarStyle::Overlay` — the titlebar is transparent and the native red/yellow/green traffic lights stay overlaid on the content (our custom titlebar leaves left space for them and hides the custom buttons). Windows/Linux use `decorations(false)` with a fully custom titlebar (drag region via `-webkit-app-region`) with minimize/maximize/close buttons wired to `/api/window-action`. The custom titlebar is hidden outside Tauri.
- **Version display**: `src-tauri/src/app_state.rs` reads `app.yml` for the version shown in the UI and falls back to `CARGO_PKG_VERSION`; `src-tauri/Cargo.toml` and `tauri.conf.json` must stay in sync at release time.
- **Settings**: the top bar gear button opens a settings panel (theme System/Light/Dark, resolved to `data-theme` on `<html>`, plus the optional "Alias" column toggle). Settings persist via `POST /api/settings` into `webui_state.json` (`app_state.rs::set_settings`), and are served back through `/api/state`; `localStorage` is not used because the local server port changes every launch.
- **Repo-ignored**: `build/`, `dist/`, `src-tauri/target/`, `node_modules`. Treat nothing in `src-tauri/target` as project source. Commit `src-tauri/Cargo.lock` and `package-lock.json` for reproducible builds.
