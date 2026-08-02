# XlsxSearcher

[English](README_EN.md) | 中文

Excel 配置表搜索工具 — 面向游戏策划，快速定位 xlsx/xls 文件中的子表和单元格数据

## 功能特性

- 🔍 **子表搜索**: 根据子表名称搜索，支持模糊、前缀、精确匹配
- 📁 **文件搜索**: 根据文件名搜索，支持模糊、前缀、精确匹配
- 🧬 **单元格搜索**: 根据单元格实际内容搜索，快速定位"某个值在哪个表里"
- 🎯 **命中定位**: 单元格搜索后自动跳到首个命中附近，并显示命中坐标
- 👁️ **Sheet 预览**: 选中结果即时预览 sheet 内容，无需打开 Excel
- ✨ **预览高亮**: 当前预览窗口内的命中单元格会高亮，当前命中位置会重点标记
- 🔎 **预览内搜索**: 支持在当前 sheet 内继续搜索，并用上一个/下一个跳转命中
- 📊 **组合搜索**: 同时按子表名称、文件名、单元格内容组合检索
- 🧭 **视图切换**: 支持按文件分组展示，也支持旧版平铺列表展示
- ↕️ **结果排序**: 支持按文件名或命中子表数排序
- 📈 **结果统计**: 状态栏实时显示当前命中文件数和子表数
- 🧾 **索引状态**: 显示当前已扫描文件数、子表数、深度索引覆盖率和待补全数量
- 🏷️ **别名映射**: 支持导入映射文件，用英文配置名搜索中文子表
- 🕘 **最近搜索**: 保存最近 15 条搜索组合，一键恢复
- 📂 **一键打开**: 双击或用按钮直接用 Excel 打开文件
- 🎯 **定位文件**: 在资源管理器/Finder 中定位并选中文件
- 📋 **复制路径**: 一键复制文件完整路径到剪贴板
- 📤 **导出结果**: 将当前搜索结果导出为 CSV 文件
- ⚡ **索引加速**: 首次扫描后建立 SQLite 索引，搜索毫秒级响应
- 🔄 **增量更新**: 重新扫描只更新有变化的文件
- 🚀 **性能优化**: 扫描和深度索引经过多轮优化，支持数万文件的高效处理
- 🛡️ **大文件保护**: 深度索引自动跳过 200MB 以上的文件，避免内存溢出
- 💾 **偏好恢复**: 记住上次扫描目录、匹配模式、排序方式和视图模式
- 🗜️ **预览折叠**: 支持 `Ctrl+`` 快捷键或按钮折叠/展开预览面板

## 环境要求

- Python 3.8+
- macOS / Windows / Linux

### 依赖安装

```bash
pip install -r requirements.txt
```

## 使用方法

```bash
python main.py
```

### CLI 使用

CLI 不依赖 GUI，复用同一个 SQLite 索引，适合脚本和自动化：

```bash
python cli.py scan ./configs
python cli.py deep-index
python cli.py search --sheet ItemConfig --format json
python cli.py hits --cell 0.41 --format json
python cli.py export result.csv --cell 新手礼包
python cli.py alias import alias.txt
python cli.py ask "新手礼包在哪个表"
python cli.py --version
```

`ask` 会先基于索引做子表名/单元格内容召回，再把命中证据交给模型回答。默认走 OpenAI 兼容接口，可通过环境变量配置：

```bash
export XLSXSEARCHER_API_KEY=sk-xxx
export XLSXSEARCHER_MODEL=gpt-4o-mini
export XLSXSEARCHER_BASE_URL=https://api.openai.com/v1
```

也兼容 `OPENAI_API_KEY` / `OPENAI_MODEL` / `OPENAI_BASE_URL`。不确定效果时先用 `python cli.py ask "问题" --dry-run` 查看检索证据和请求内容。

### 操作流程

1. 点击 **「选择目录」** 选择要扫描的文件夹
2. 程序自动扫描目录下所有 `xlsx` / `xlsm` / `xls` 文件并建立索引
3. 点击 **「深度索引」** 提取所有 sheet 的单元格内容（仅需做一次，后续增量扫描不受影响）
4. 在搜索框输入关键词进行搜索：
   - **子表名称**: 搜索 Sheet 名称
   - **文件名**: 搜索文件名
   - **单元格**: 搜索实际的单元格数值
5. 切换搜索选项：
   - **匹配模式**: 模糊匹配 / 前缀匹配 / 精确匹配
   - **排序方式**: 文件名 A-Z / 文件名 Z-A / 子表数最多 / 子表数最少
   - **结果视图**: 分组视图 / 列表视图
   - **最近搜索**: 快速恢复最近使用过的搜索条件
6. **点击任意结果** → 下方预览面板显示对应 sheet 内容
   - 如果当前用了**单元格搜索**，会自动跳到首个命中附近并高亮命中单元格
   - 预览标题会显示命中数和当前命中坐标
   - 可在预览面板里继续输入关键词，使用「上一个 / 下一个」在当前 sheet 内跳转命中
   - `Ctrl+`` 或点击 「▾ 折叠预览」按钮可收起/展开预览面板
7. 使用底部按钮操作文件：
   - **打开文件**: 用默认程序打开文件
   - **定位文件**: 在文件管理器中选中文件
   - **复制路径**: 复制文件路径到剪贴板
   - **导出结果**: 导出当前搜索结果到 CSV 文件

其他功能：
- **重新扫描**: 重新扫描当前选择的目录（只更新有变化的文件）
- **清空索引**: 清除所有已建立的索引数据
- **深度索引**: 提取所有 sheet 的单元格内容以支持单元格搜索
- **索引状态提示**: 如果单元格搜索没有结果，状态栏会提示是否仍有未完成深度索引的子表
- **别名映射**: 点击「导入映射」选择 `.txt` 映射文件，即可用英文配置名搜索中文子表。文件格式为 `英文配置名 子表名1 子表名2 ...`，以 `#` 开头为注释，例如：
  ```
  # 界面文本配置
  TextConfig 界面文本 UI Text
  # 道具配置
  ItemConfig 道具配置 Item Config
  ```

## 项目结构

```
XlsxSearcher/
├── main.py              # 程序入口
├── cli.py               # CLI 入口（扫描/搜索/导出/问答）
├── app.yml              # 应用配置（版本号、数据目录等）
├── requirements.txt     # 依赖
├── icons/               # 应用图标
├── core/
│   ├── indexer.py       # SQLite 索引管理
│   ├── scanner.py       # xlsx/xls 文件扫描
│   └── searcher.py      # 搜索逻辑
├── gui/
│   └── app.py           # PyQt5 主界面
└── utils/
    └── file_utils.py    # 文件操作工具
```

## 打包发布

### macOS

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.png \
  --add-data "icons/app_icon.png:icons" \
  --add-data "app.yml:." \
  main.py
pyinstaller --onefile --name XlsxSearcherCLI \
  --add-data "app.yml:." \
  cli.py
```

生成的 `XlsxSearcher.app` 和 CLI 可执行文件 `XlsxSearcherCLI` 都在 `dist` 目录下，`.app` 解压后双击运行。

### Windows

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.ico \
  --add-data "icons/app_icon.png;icons" \
  --add-data "app.yml;." \
  main.py
pyinstaller --onefile --name XlsxSearcherCLI \
  --add-data "app.yml;." \
  cli.py
```

生成的 `XlsxSearcher.exe` 和 CLI 可执行文件 `XlsxSearcherCLI.exe` 都在 `dist` 目录下。

### Linux

```bash
pyinstaller --onefile --windowed --name XlsxSearcher \
  --icon icons/app_icon.png \
  --add-data "icons/app_icon.png:icons" \
  --add-data "app.yml:." \
  main.py
pyinstaller --onefile --name XlsxSearcherCLI \
  --add-data "app.yml:." \
  cli.py
```

生成的可执行文件 `XlsxSearcher` 和 CLI 可执行文件 `XlsxSearcherCLI` 都在 `dist` 目录下。

## 配置

应用配置文件 `app.yml` 位于项目根目录，可自定义版本号、图标路径和数据存储目录：

```yaml
# XlsxSearcher 应用配置
app:
  name: XlsxSearcher
  version: "1.4.3"          # 版本号，发布时修改此处
  icon: icons/app_icon.png   # 运行时窗口图标
  data_dir: ~/.local/XlsxSearcher  # 数据库和日志存放目录
```

## 数据存储

索引数据库和日志默认保存在 `app.yml` 中 `data_dir` 指定的目录下：

- **索引数据库**: `~/.local/XlsxSearcher/index.db`
- **应用日志**: `~/.local/XlsxSearcher/app.log`
- **本地偏好**: 通过 `QSettings` 存储（macOS: `~/Library/Preferences/`）

## 许可证

MIT License
