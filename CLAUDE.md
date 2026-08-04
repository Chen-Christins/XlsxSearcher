# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
```bash
# Install runtime dependencies
pip install -r requirements.txt

# Build the TypeScript UI once
cd webui && npm install && npm run build && cd ..

# Start the app (GUI, requires display)
python main.py

# Quick smoke-check that all modules compile
python -m compileall main.py core utils web webui
```

There is no test suite, linter config, or typecheck setup in this repo.

### Packaging
```bash
# PyInstaller is NOT in requirements.txt — install it separately for packaging
pip install pyinstaller

# Build standalone app (macOS / Linux separator is `:`, Windows is `;`)
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.ico \
  --add-data "icons/app_icon.png:icons" \
  --add-data "webui/dist:webui/dist" \
  --collect-all webview \
  main.py
```

CI (`.github/workflows/build.yml`) builds on push to `master`, PRs to `master`, version tags (`v*`), and manual dispatch. The matrix builds macOS, Windows, and Ubuntu, each with `python-version: '3.11'`. On tag pushes it creates a draft GitHub Release and attaches per-platform `.zip` artifacts.

## Architecture

```
main.py              ← thin entrypoint, calls run_desktop()
webui/desktop.py     ← PyWebView desktop shell; loads the built frontend
webui/web.py         ← browser-only mode; starts the same local API
web/server.py        ← local HTTP API; owns IndexManager/XlsxScanner/Searcher
webui/src            ← TypeScript UI source, compiled to webui/dist
core/scanner.py      ← file discovery, sheet-name extraction, cell-content reading, hit-finding
core/indexer.py      ← SQLite schema, CRUD, all search queries
core/searcher.py     ← thin orchestration layer: resolves aliases → delegates to IndexManager.search()
core/alias_parser.py ← parses alias-mapping .txt files (two formats, see below)
utils/file_utils.py  ← OS-specific open/reveal/clipboard (win32/darwin/linux)
```

`web/server.py` constructs `IndexManager`, `XlsxScanner`, and `Searcher` directly — there is no DI container or service layer. The local HTTP server is a `ThreadingHTTPServer`; the TypeScript UI talks to it over JSON.

### Threading model
- **Scan job** — a daemon thread calls `scanner.scan_directory_incremental()`; sheet-name extraction runs in `ThreadPoolExecutor(max_workers=8)`, but all SQLite writes happen on the job thread via `index_manager.upsert_files_batch()`.
- **Deep index job** — a daemon thread groups `cell_text IS NULL` sheets by file and uses a `multiprocessing.Pool` (max 4 workers) with `extract_file_cell_texts_pool`; SQLite batch updates stay in the job thread.
- **Search requests** — run on the HTTP request thread and stay off the UI thread; the frontend owns request supersession via sequence counters.
- **Preview requests** — run `scanner.read_sheet_with_hits()` on the HTTP request thread so large sheets never block the UI; the frontend discards stale responses with a preview sequence token.

### SQLite connections
`IndexManager` keeps one long-lived connection **per thread** (`threading.local`, lazily created via `_conn()`), all in WAL mode. Do NOT call `sqlite3.connect(self.db_path)` directly inside `IndexManager` methods — use `self._conn()` and omit `conn.close()` (the connection is reused). Writes commit explicitly; reads are autocommit. `sheets.cell_text` is backed by an FTS5 trigram external-content table (`sheets_fts`) kept in sync by AFTER INSERT/UPDATE/DELETE triggers, so upsert / deep-index need no FTS-specific code.

### Data flow
1. User selects a directory through the desktop dialog (or a directory-picker fallback) → a scan job walks it with `os.scandir` (DFS, skips hidden dirs by `.` prefix), extracts sheet names from `xl/workbook.xml` via compiled regex (fast path) or falls back to openpyxl/xlrd.
2. Sheet names are written to `~/.local/XlsxSearcher/index.db` (SQLite, WAL mode). The DB has three tables: `xlsx_files`, `sheets`, `sheet_aliases`.
3. "Deep index" extracts cell text from every sheet using openpyxl read-only mode, stored in `sheets.cell_text`. Files >200MB are skipped.
4. Searches query the SQLite index directly — results are grouped by file in `_fetch_grouped_results()`. Cell-content search (>=3-char keyword) uses the FTS5 trigram index (`sheets_fts MATCH`); shorter keywords fall back to `LIKE`.
5. Sheet preview reads a 20-row × 50-col window via `scanner.read_sheet_with_hits()` on an HTTP request thread, with window positioning around hit cells. The read is single-pass streaming (bounded memory), not a full-sheet load.

### Match modes
Three modes, applied at the SQL query level for sheet-name/filename: `fuzzy` (`LIKE '%keyword%'`), `prefix` (`LIKE 'keyword%'`), `exact` (`= keyword`). All use `COLLATE NOCASE`. For cell-content search, FTS5 trigram handles the fuzzy (substring) case directly; `prefix`/`exact` FTS-narrow then apply the corresponding LIKE/`LOWER()=` post-filter to preserve semantics.

### Alias mapping
`core/alias_parser.py` supports two formats in `.txt` mapping files:
- **Standard** (recommended): `EnglishConfigName SheetName1 SheetName2 ...`
- **Legacy**: `call <script> <AliasNames...> <SheetNames...>` (Cartesian product of aliases × sheet names)

Comments are `#`, `::`, or `REM`. The parser tries encodings UTF-8-SIG → UTF-8 → GBK → GB18030.

### File format support
`.xlsx`, `.xlsm` (modern ZIP-based, read via `python-calamine` Rust library with openpyxl fallback) and `.xls` (legacy binary, via xlrd). Sheet-name extraction uses a compiled regex on `xl/workbook.xml` raw bytes (~2-3x faster than ET.parse).

## Key details

- **Index database**: `~/.local/XlsxSearcher/index.db` on all platforms. `clear_index()` wipes this user-level database (affects the app globally, not just the workspace).
- **Preview panel**: bound to `Ctrl+`` / `Cmd+`` toggle. Shows rows starting from `start_row`/`start_col` around the first hit. Hit cells are highlighted; the current hit is marked distinctly.
- **Search history**: not yet migrated to the TypeScript UI.
- **Scanner DFS**: uses `os.scandir` with an explicit stack (not `os.walk`), skips directories whose name starts with `.`. Files are sorted by full path before processing for better disk locality.
- **Progress reporting**: batch interval is every 16 files to reduce GUI signal overhead.
- **Upsert batch writes**: `upsert_files_batch()` preserves existing `cell_text` across re-scans by querying old values before the DELETE+INSERT cycle. When a file's sheet-name list is unchanged since the last scan, the sheets rows are NOT rebuilt — only `xlsx_files.modified_time` is updated, so deep-indexed `cell_text` is trivially retained. Using `add_file()` instead of the batch method on updated files will wipe `cell_text` to NULL.
- **File reading**: xlsx/xlsm cell reading uses `python-calamine` (Rust-level XML parsing, ~5-10x faster than openpyxl). Falls back to openpyxl automatically if calamine fails. `.xls` format still uses xlrd.
- **Combined file open**: `XlsxScanner.read_sheet_with_hits()` opens a file once and returns hits + preview data + header row simultaneously. Use this instead of separate `find_sheet_matches()` + `read_sheet_preview()` calls. The old methods still exist and delegate to the combined method internally.
- **Preview rendering**: `webui/src/app.ts` has `renderPreview()` that takes already-loaded data (no file I/O). The frontend calls the `/api/preview` endpoint; do NOT call `scanner.read_sheet_with_hits()` synchronously on the UI thread.
- **Repo-ignored**: `.venv/`, `build/`, `dist/`, `*.spec`, `__pycache__/`. Treat nothing in `.venv` as project source.
- **Python version**: CI uses 3.11. Runtime requires 3.8+.
