# AGENTS.md

## Run And Verify
- Install runtime deps with `pip install -r requirements.txt`.
- Install and build the web UI with `cd webui && npm install && npm run build`.
- Start the app with `python main.py`.
- There is no repo test, lint, or typecheck config. The safest quick verification is `python -m compileall main.py core utils web webui`, then a manual smoke run of `python main.py` if the environment can launch a GUI.

## Packaging
- `pyinstaller` is not listed in `requirements.txt`; install it separately before packaging, matching CI: `python -m pip install --upgrade pip && pip install -r requirements.txt && pip install pyinstaller`.
- The TypeScript UI must be built before packaging (`cd webui && npm install && npm run build`). CI runs this automatically and bundles `webui/dist` plus `--collect-all webview`.
- CI builds the app from `main.py` with `pyinstaller --onefile --windowed --name ... main.py` on macOS, Windows, and Ubuntu. Trust `.github/workflows/build.yml` over the README packaging examples.

## Architecture
- `main.py` is only a thin entrypoint; the current desktop client wiring lives in `webui/desktop.py` (PyWebView shell) and `web/server.py` (local HTTP API).
- `webui/web.py` starts the same local API in browser-only mode; the desktop client can also open the current URL in the system browser.
- The TypeScript UI lives in `webui/src` and is compiled to `webui/dist` with `npm run build`.
- `web/server.py` constructs `IndexManager`, `XlsxScanner`, and `Searcher` and exposes them through a local API; the UI never touches `core/` directly.
- `core/scanner.py` owns recursive file discovery and sheet-name extraction.
- `core/indexer.py` owns the SQLite schema and all search queries.
- `utils/file_utils.py` contains the OS-specific open / reveal / clipboard behavior.

## Data And Scan Quirks
- The index database is not stored in the repo. `IndexManager` writes `index.db` under the user home directory hidden folder for all platforms: `~/.local/XlsxSearcher/index.db`.
- `clear_index()` wipes that user-level database, so it affects the local app cache outside the workspace.
- The scanner supports both `.xlsx` and `.xlsm` files even though the README mostly says xlsx.
- Directory scans skip hidden directories by name (`.` prefix).
- Sheet names are read from `xl/workbook.xml` via `zipfile`/XML first; `openpyxl` is only the slow fallback for unreadable files.
- Incremental scans parallelize sheet extraction with `ThreadPoolExecutor`, but SQLite writes still happen on the main thread via `IndexManager.add_file()`.

## Repo State
- `.venv/`, `build/`, `dist/`, `*.spec`, and `__pycache__/` are ignored. Avoid treating anything inside `.venv` as project source when searching the repo.
