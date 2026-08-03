# AGENTS.md

## Run And Verify
- Install runtime deps with `pip install -r requirements.txt`.
- Start the app with `python main.py`.
- There is no repo test, lint, or typecheck config. The safest quick verification is `python -m compileall main.py core gui utils`, then a manual smoke run of `python main.py` if the environment can launch a GUI.

## Packaging
- `pyinstaller` is not listed in `requirements.txt`; install it separately before packaging, matching CI: `python -m pip install --upgrade pip && pip install -r requirements.txt && pip install pyinstaller`.
- CI builds the GUI from `main.py` with `pyinstaller --onefile --windowed --name ... main.py` and the CLI from `cli.py` with `pyinstaller --onefile --name XlsxSearcherCLI ... cli.py` on macOS, Windows, and Ubuntu. Trust `.github/workflows/build.yml` over the README packaging examples.

## Agent CLI Usage
- The CLI entrypoint is `python cli.py`; the packaged executable is `XlsxSearcherCLI` (Windows: `XlsxSearcherCLI.exe`).
- The CLI shares the GUI's SQLite index at `~/.local/XlsxSearcher/index.db`; no `--db` is needed unless a different index is intended.
- Use `python cli.py search --sheet <keyword> --format json` for deterministic search answers; results go to stdout, progress/warnings go to stderr, and exit code 0 means success.
- Available commands: `scan`, `deep-index`, `search`, `export`, `alias import|list`, `stats`, `ask`, and `version`/`--version`.
- `ask` retrieves matching sheets/cells first and then calls an OpenAI-compatible endpoint; prefer `python cli.py ask "<question>" --dry-run` to inspect retrieval evidence without making a network request.
- For `ask`, configure `OPENAI_API_KEY`/`XLSXSEARCHER_API_KEY`, `OPENAI_MODEL`/`XLSXSEARCHER_MODEL`, and `OPENAI_BASE_URL`/`XLSXSEARCHER_BASE_URL`.

## Architecture
- `main.py` is only a thin entrypoint; the real app wiring lives in `gui/app.py`.
- `gui/app.py` constructs `IndexManager`, `XlsxScanner`, and `Searcher` directly inside `XlsxSearcherApp`; there is no separate service layer.
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
