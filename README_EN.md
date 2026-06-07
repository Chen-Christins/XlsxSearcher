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
- 🏷️ **Alias Mapping**: Import mapping files to search Chinese sheet names using English config names
- 🕘 **Recent Searches**: Save the last 15 search combinations, one-click restore
- 📂 **Open File**: Double-click or use button to open file directly in Excel
- 🎯 **Locate in Finder**: Reveal and select the file in Explorer/Finder
- 📋 **Copy Path**: One-click copy full file path to clipboard
- 📤 **Export CSV**: Export current search results to a CSV file
- ⚡ **Index Acceleration**: Builds SQLite index on first scan for millisecond-level search responses
- 🔄 **Incremental Update**: Re-scan only updates changed files
- 💾 **Preference Persistence**: Remembers last scanned directory, match mode, sort order, and view mode
- 🗜️ **Collapse Preview**: Use `Ctrl+`` shortcut or button to collapse/expand the preview panel

## Requirements

- Python 3.8+
- macOS / Windows / Linux

### Install Dependencies

```bash
pip install -r requirements.txt
```

## Usage

```bash
python main.py
```

### Workflow

1. Click **"Select Directory"** to choose a folder to scan
2. The program auto-scans all `xlsx` / `xlsm` / `xls` files in the directory and builds an index
3. Click **"Deep Index"** to extract cell contents from all sheets (one-time operation; subsequent incremental scans are unaffected)
4. Enter keywords in the search box:
   - **Sheet Name**: Search by sheet name
   - **File Name**: Search by file name
   - **Cell Value**: Search by actual cell content
5. Search options:
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
├── main.py              # Entry point
├── requirements.txt     # Dependencies
├── icons/               # App icons
├── core/
│   ├── indexer.py       # SQLite index management
│   ├── scanner.py       # xlsx/xls file scanning
│   └── searcher.py      # Search logic
├── gui/
│   └── app.py           # PyQt5 main UI
└── utils/
    └── file_utils.py    # File operation utilities
```

## Build & Distribute

### macOS

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.png \
  --add-data "icons/app_icon.png:icons" \
  main.py
```

The generated `.app` is in the `dist` directory. Extract and double-click to run.

### Windows

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.ico \
  --add-data "icons/app_icon.png;icons" \
  main.py
```

The generated `.exe` is in the `dist` directory.

### Linux

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.png \
  --add-data "icons/app_icon.png:icons" \
  main.py
```

The generated executable is in the `dist` directory.

## Data Storage

The index database and local preferences are stored in the user directory:

- **Index Database**: `~/.local/XlsxSearcher/index.db`
- **Preferences**: Stored via `QSettings` (macOS: `~/Library/Preferences/`)

## License

MIT License
