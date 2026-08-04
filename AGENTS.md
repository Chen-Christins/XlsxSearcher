# AGENTS.md

## Run And Verify
- Build the web UI first: `npm --prefix webui install && npm --prefix webui run build`.
- Install the Tauri CLI: `npm install` at the repo root (installs `@tauri-apps/cli`).
- Start the desktop app with `npm run tauri -- dev`.
- There is no repo test, lint, or typecheck config. The quickest verification is `cd webui && npm run build` (TypeScript), then `cargo build` in `src-tauri` (Rust), then a manual smoke run of `npm run tauri -- dev` if the environment can launch a GUI.

## Packaging
- Tauri bundles are produced with `npm run tauri -- build` (release). Run it from the repo root; `src-tauri/tauri.conf.json` is auto-discovered and `beforeBuildCommand` runs `npm run build` with `cwd: ../webui`.
- The web UI must be built before packaging; the Tauri `beforeBuildCommand` handles it, and CI also builds `webui/dist` explicitly.
- CI builds with `npm run tauri -- build --bundles ...` on macOS (universal-apple-darwin), Windows (nsis), and Ubuntu (deb, appimage). Trust `.github/workflows/build.yml` over the README packaging examples.
- `webui/dist` is embedded into the Rust binary via `include_dir!` in `src-tauri/src/server.rs`; do not rely on a runtime filesystem path for the UI in packaged builds.

## Architecture
- The desktop client is a Tauri 2 (Rust) app in `src-tauri/`. `main.rs` boots a local axum HTTP server on `127.0.0.1:0`, then opens a frameless webview pointed at that URL.
- `src-tauri/src/server.rs` exposes the local API (state/search/preview/scan/deep-index/clear-index/action/aliases/export/dialogs/window-action/open-browser) and serves the embedded `webui/dist` as the fallback route.
- The TypeScript UI lives in `webui/src` and is compiled to `webui/dist` with `npm run build`. It talks to the HTTP API only; it never touches `src-tauri` directly.
- Native operations are bridged through the API: the frontend uses `POST /api/choose-directory`, `/api/choose-alias-file`, `/api/choose-export-file`, `/api/window-action`, `/api/open-browser`, with web fallbacks (file-input / blob download / `window.open`) for plain-browser mode.
- `src-tauri/src/db.rs` owns the SQLite schema (FTS5 trigram, sheet aliases) and all search queries. `src-tauri/src/scanner.rs` owns file discovery, sheet-name extraction, and cell reading via `calamine`.

## Data And Scan Quirks
- The index database is not stored in the repo. The app writes `index.db` under the user home directory hidden folder for all platforms: `~/.local/XlsxSearcher/index.db`.
- `clear_index()` wipes that user-level database, so it affects the local app cache outside the workspace.
- The scanner supports `.xlsx`, `.xlsm`, and `.xls` (via calamine) even though the README mostly says xlsx.
- Directory scans skip hidden directories by name (`.` prefix).
- Sheet names are read from `xl/workbook.xml` via `zip`/regex first; `calamine` is the fallback for unreadable or `.xls` files.
- Deep indexing and scans parallelize extraction with `rayon`, but SQLite writes go through the r2d2 pool (`src-tauri/src/db.rs`).
- `init_fts` rebuilds the FTS tables on startup so databases created by the old Python backend are re-indexed correctly.

## Repo State
- `build/`, `dist/`, `src-tauri/target/`, and `node_modules` are ignored. Avoid treating anything inside `src-tauri/target` as project source when searching the repo.
- `src-tauri/Cargo.lock` and the root `package-lock.json` are committed for reproducible builds.
