# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

### Development
```bash
# Install runtime dependencies
pip install -r requirements.txt

# Start the app (GUI, requires display)
python main.py

# Quick smoke-check that all modules compile
python -m compileall main.py core gui utils
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
  main.py
```

CI (`.github/workflows/build.yml`) builds on push to `master`, PRs to `master`, version tags (`v*`), and manual dispatch. The matrix builds macOS, Windows, and Ubuntu, each with `python-version: '3.11'`. On tag pushes it creates a draft GitHub Release and attaches per-platform `.zip` artifacts.

## Architecture

```
main.py              ← thin entrypoint, calls run_app()
gui/app.py           ← ALL UI and app wiring (PyQt5, ~66k LOC single file)
core/scanner.py      ← file discovery, sheet-name extraction, cell-content reading, hit-finding
core/indexer.py      ← SQLite schema, CRUD, all search queries
core/searcher.py     ← thin orchestration layer: resolves aliases → delegates to IndexManager.search()
core/alias_parser.py ← parses alias-mapping .txt files (two formats, see below)
utils/file_utils.py  ← OS-specific open/reveal/clipboard (win32/darwin/linux)
```

`XlsxSearcherApp.__init__` constructs `IndexManager`, `XlsxScanner`, and `Searcher` directly — there is no DI container or service layer. All long-running work (scan, deep-index, search) runs on `QThread` subclasses (`ScanWorker`, `DeepIndexWorker`, `SearchWorker`) defined in `gui/app.py`.

### Threading model
- **ScanWorker** — calls `scanner.scan_directory_incremental()`; sheet-name extraction runs in `ThreadPoolExecutor(max_workers=8)`, but all SQLite writes happen on the worker's main thread via `index_manager.upsert_files_batch()`.
- **DeepIndexWorker** — groups `cell_text IS NULL` sheets by file, processes with `ThreadPoolExecutor(max_workers=2)` (kept low to limit openpyxl memory), each thread creates its own `XlsxScanner` instance.
- **SearchWorker** — runs SQLite queries in a background thread to keep the UI responsive; supports cooperative cancellation via `_cancelled` flag (NEVER use `QThread.terminate()` — it can leak SQLite connections).

### Data flow
1. User selects a directory → `ScanWorker` walks it with `os.scandir` (DFS, skips hidden dirs by `.` prefix), extracts sheet names from `xl/workbook.xml` via compiled regex (fast path) or falls back to openpyxl/xlrd.
2. Sheet names are written to `~/.local/XlsxSearcher/index.db` (SQLite, WAL mode). The DB has three tables: `xlsx_files`, `sheets`, `sheet_aliases`.
3. "Deep index" extracts cell text from every sheet using openpyxl read-only mode, stored in `sheets.cell_text`. Files >200MB are skipped.
4. Searches query the SQLite index directly — results are grouped by file in `_fetch_grouped_results()`.
5. Sheet preview reads up to 20 rows × 50 cols via openpyxl/xlrd, with window positioning around hit cells.

### Match modes
Three modes, applied at the SQL query level: `fuzzy` (`LIKE '%keyword%'`), `prefix` (`LIKE 'keyword%'`), `exact` (`= keyword`). All use `COLLATE NOCASE`.

### Alias mapping
`core/alias_parser.py` supports two formats in `.txt` mapping files:
- **Standard** (recommended): `EnglishConfigName SheetName1 SheetName2 ...`
- **Legacy**: `call <script> <AliasNames...> <SheetNames...>` (Cartesian product of aliases × sheet names)

Comments are `#`, `::`, or `REM`. The parser tries encodings UTF-8-SIG → UTF-8 → GBK → GB18030.

### File format support
`.xlsx`, `.xlsm` (modern ZIP-based, read via `python-calamine` Rust library with openpyxl fallback) and `.xls` (legacy binary, via xlrd). Sheet-name extraction uses a compiled regex on `xl/workbook.xml` raw bytes (~2-3x faster than ET.parse).

## Key details

- **Index database**: `~/.local/XlsxSearcher/index.db` on all platforms. `clear_index()` wipes this user-level database (affects the app globally, not just the workspace).
- **Preview panel**: bound to `Ctrl+`` toggle. Shows rows starting from `start_row`/`start_col` around the first hit. Hit cells are highlighted; the current hit is marked distinctly.
- **Search history**: last 15 search combinations persisted via `QSettings` (macOS: `~/Library/Preferences/com.XlsxSearcher.XlsxSearcher.plist`).
- **Scanner DFS**: uses `os.scandir` with an explicit stack (not `os.walk`), skips directories whose name starts with `.`. Files are sorted by full path before processing for better disk locality.
- **Progress reporting**: batch interval is every 16 files to reduce GUI signal overhead.
- **Upsert batch writes**: `upsert_files_batch()` preserves existing `cell_text` across re-scans by querying old values before the DELETE+INSERT cycle. Using `add_file()` instead of the batch method on updated files will wipe `cell_text` to NULL.
- **File reading**: xlsx/xlsm cell reading uses `python-calamine` (Rust-level XML parsing, ~5-10x faster than openpyxl). Falls back to openpyxl automatically if calamine fails. `.xls` format still uses xlrd.
- **Combined file open**: `XlsxScanner.read_sheet_with_hits()` opens a file once and returns hits + preview data + header row simultaneously. Use this instead of separate `find_sheet_matches()` + `read_sheet_preview()` calls. The old methods still exist and delegate to the combined method internally.
- **Preview rendering**: `gui/app.py` has `_render_preview_table()` that takes already-loaded data (no file I/O). `_update_preview()` is a thin wrapper that loads data via `read_sheet_with_hits(keyword=None)` then calls `_render_preview_table()`. Use `_render_preview_table()` when you already have the data from a prior combined call.
- **Repo-ignored**: `.venv/`, `build/`, `dist/`, `*.spec`, `__pycache__/`. Treat nothing in `.venv` as project source.
- **Python version**: CI uses 3.11. Runtime requires 3.8+.
