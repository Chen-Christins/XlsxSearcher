# XlsxSearcher

English | [中文](README.md)

Excel Config Table Search Tool — Quickly locate sheets and cell data in xlsx/xls files, designed for game designers

## Features

- 🔍 **Sheet Search**: Search by sheet name, supports fuzzy, prefix, and exact matching
- 📁 **File Search**: Search by file name, supports fuzzy, prefix, and exact matching
- 🧬 **Cell Search**: Search by actual cell content — quickly find "which table contains this value"
- 🎯 **Hit Navigation**: Auto-jump to the first hit after cell search, with hit coordinate display
- 👁️ **Sheet Preview**: Preview sheet content instantly upon selection, no need to open Excel
- ✨ **Preview Highlight**: Matching cells in the current preview are highlighted; the current hit is marked with emphasis
- 🔎 **In-Preview Search**: Continue searching within the current sheet, with Previous/Next hit navigation
- 📊 **Combined Search**: Search by sheet name, file name, and cell content simultaneously
- 🧭 **View Switch**: Toggle between grouped-by-file view and flat list view
- ↕️ **Result Sorting**: Sort results by file name or number of matching sheets
- 📈 **Result Statistics**: Status bar shows real-time count of matching files and sheets
- 🧾 **Index Status**: Displays scanned file count, sheet count, deep index coverage, and pending count
- 🏷️ **Alias Mapping**: Import mapping files to search Chinese sheet names using English config names, and show each sheet's aliases in results
- ⚙️ **Settings**: Gear button switches theme (System / Light / Dark; System follows the OS live) and toggles the "Alias" column; settings are persisted
- 🕘 **Recent Searches**: Save the last 15 search combinations, one-click restore
- 📂 **Open File**: Double-click or use button to open file directly in Excel
- 🎯 **Locate in Finder**: Reveal and select the file in Explorer/Finder
- 📋 **Copy Path**: One-click copy full file path to clipboard
- 📤 **Export CSV**: Export current search results to a CSV file
- ⚡ **Index Acceleration**: Builds SQLite index on first scan for millisecond-level search responses
- 🔄 **Incremental Update**: Re-scan only updates changed files
- 🚀 **Optimized Performance**: Multi-round optimization for scanning and deep indexing, efficiently handling tens of thousands of files
- 🛡️ **Large File Protection**: Deep indexing automatically skips files larger than 200MB to prevent out-of-memory errors
- 💾 **Preference Persistence**: Remembers last scanned directory, match mode, sort order, and view mode
- 🗜️ **Collapse Preview**: Use `Ctrl+`` shortcut or button to collapse/expand the preview panel

## Requirements

- macOS / Windows / Linux
- Node.js 18+ (builds the Web UI), Rust stable (builds the desktop shell)

### Install Dependencies

```bash
# Install root deps (includes @tauri-apps/cli)
npm install
# Install and build the Web UI
cd webui && npm install && npm run build && cd ..
```

## Usage

### Desktop client

```bash
npm run tauri -- dev
```

### Web Interface Mode

The desktop client has a **Web UI** button in the top bar that opens the same interface in the system browser.
```

### Workflow

1. Click **"Select Directory"** to choose a folder to scan
2. The program auto-scans all `xlsx` / `xlsm` / `xls` files in the directory, builds an index, and auto-extracts cell contents from new files (automatic deep index), so **cell search works out of the box**; the **"Deep Index"** button can re-index manually, and unchanged re-scans skip it
3. Enter keywords in the search box:
   - **Sheet Name**: Search by sheet name
   - **File Name**: Search by file name
   - **Cell Value**: Search by actual cell content
4. Search options:
   - **Match Mode**: Fuzzy / Prefix / Exact
   - **Sort By**: File Name A-Z / File Name Z-A / Most Sheets / Fewest Sheets
   - **View**: Group View / List View
   - **Recent**: Quickly restore recently used search criteria
6. **Click any result** → the preview panel below shows the sheet content
   - If **cell search** is active, it auto-jumps to the first hit and highlights matching cells
   - The preview header shows hit count and current hit coordinates
   - You can continue searching within the preview panel, using **Previous / Next** to jump between hits in the current sheet
   - `Ctrl+`` or click **"▾ Collapse Preview"** to toggle the preview panel
7. Use bottom buttons for file operations:
   - **Open File**: Open file with default program
   - **Locate File**: Reveal file in file manager
   - **Copy Path**: Copy file path to clipboard
   - **Export**: Export current search results to CSV

Other features:
- **Rescan**: Re-scan current directory (updating only changed files)
- **Clear Index**: Clear all index data
- **Deep Index**: Extract cell contents from all sheets to enable cell search
- **Index Status Hint**: If cell search yields no results, the status bar indicates if there are sheets still pending deep indexing
- **Alias Mapping**: Click "Import Mapping" to select a `.txt` mapping file, allowing you to search Chinese sheet names using English config names. The file format is `EnglishConfigName SheetName1 SheetName2 ...`, with `#` for comments, e.g.:
  ```
  # UI Text Config
  TextConfig UI Text 界面文本
  # Item Config
  ItemConfig Item Config 道具配置
  ```

## Project Structure

```
XlsxSearcher/
├── package.json         # Root config (tauri script, @tauri-apps/cli)
├── app.yml              # App config (version, data dir, etc.)
├── src-tauri/           # Tauri desktop client (Rust)
│   ├── src/
│   │   ├── main.rs      # Entry: local HTTP server + webview (native macOS traffic lights)
│   │   ├── server.rs    # Local HTTP API + embedded frontend assets
│   │   ├── db.rs        # SQLite index management (FTS5)
│   │   └── scanner.rs   # xlsx/xlsm/xls scanning & cell reading
│   └── tauri.conf.json  # Tauri config (bundling, icons, version)
└── webui/
    ├── src/             # TypeScript frontend source
    └── dist/            # Built frontend static files
```

## Build & Distribute

Install dependencies and build the Web UI first (`npm install` + `cd webui && npm install && npm run build`).

### macOS

```bash
npm run tauri -- build --bundles app,dmg
```

The generated `.app` / `.dmg` is in `src-tauri/target/release/bundle/`.

### Windows

```bash
npm run tauri -- build --bundles nsis
```

The generated installer is in `src-tauri/target/release/bundle/nsis/`.

### Linux

```bash
npm run tauri -- build --bundles deb,appimage
```

The generated packages are in `src-tauri/target/release/bundle/`.

CI builds all of the above automatically on macOS, Windows, and Ubuntu and publishes them to GitHub Releases (see `.github/workflows/build.yml`).

## Configuration

Application config `app.yml` is located at the project root. You can customize version and data directory:

```yaml
# XlsxSearcher application configuration
app:
  name: XlsxSearcher
  version: "1.4.4"          # App version, update here before release
  data_dir: ~/.local/XlsxSearcher  # Index DB and last-scanned directory location
```

> The Tauri shell reads the version from `app.yml` for display; the packaged version comes from `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`. Keep all three in sync when releasing.

## Data Storage

Index database is stored under the `data_dir` path configured in `app.yml`:

- **Index Database**: `~/.local/XlsxSearcher/index.db`
- **Last Scanned Directory**: `~/.local/XlsxSearcher/webui_state.json`

## License

MIT License
