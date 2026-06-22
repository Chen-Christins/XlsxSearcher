# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added
- **FTS5 全文索引**：`sheets.cell_text` 新增 trigram 分词的 FTS5 倒排索引（外部内容表 + 触发器自动同步）。单元格内容搜索从 `LIKE '%kw%'` 全表扫描改为倒排索引命中，深度索引后搜索提速数倍；`prefix`/`exact` 模式在 FTS 缩小候选集后再用 LIKE 收紧，短关键字（<3 字符）自动回退 LIKE，行为与旧版一致
- **预览异步加载**：新增 `PreviewWorker`，预览读取（命中查找 + 窗口数据 + 表头）移至后台线程。选中大表不再阻塞 UI；多个请求通过 token 取舍，旧结果不会回写
- **性能对比测试**：新增 `benchmarks/bench.py`，覆盖三项优化的正确性断言与耗时/内存对比

### Improved
- **预览读取流式化**：`read_sheet_with_hits` 由"整表载入内存再切片"改为单遍流式，只保留有界滚动窗口（约 `preview_rows` 行）+ 有界命中列表。大表预览峰值内存从 ~42MB 降至接近 0，耗时下降约 7-8x
- **SQLite 连接复用**：`IndexManager` 改为每线程长连接（`threading.local`），不再每次操作 connect/close；配合已有的 WAL 模式降低反复建连开销
- **upsert 跳过未变 sheets**：重新扫描时若某文件的 sheet 名列表与索引完全一致，仅更新 mtime，跳过 sheets 的 DELETE+INSERT 重建，避免搬运已深度索引的 `cell_text`（二次扫描耗时大幅下降）
- 移除冗余索引 `idx_filepath`（`filepath` 已有 UNIQUE 约束自带索引）

## [1.4.1] - 2026-06-20

## What's Changed

### Added
- 新增 **python-calamine 加速解析**（Rust 级 XML 解析），单元格内容提取和 sheet 预览读取速度提升 5-10 倍，openpyxl 作为自动回退
- 新增 **CLAUDE.md** AI 开发指引文档，记录项目架构、线程模型、数据流和关键实现细节

### Improved
- **预览表头改进**：预览表格横表头（列标题）从原本仅显示列字母（A/B/C...）改为优先使用 Excel 第 1 行实际单元格内容作为表头，无内容时回退为列字母
- **文件 I/O 合并优化**：`find_sheet_matches`（命中查找）和 `read_sheet_preview`（预览读取）合并为单次文件打开的 `read_sheet_with_hits`，一次加载同时返回命中、预览数据和表头，减少重复 open/parse 开销
- 预览渲染逻辑拆分为 `_render_preview_table`（纯数据→表格渲染），与 `_update_preview`（数据加载→渲染）职责分离

### Fixed
- 修复预览面板表头缺失问题 — `_update_preview` 原先只显示列字母，未读取 Excel 首行作为实际列标题
- 修复预览 1-20 行与表头重复的问题 — 数据起始行统一从 row 2 开始（跳过表头行），避免首行数据与表头重复显示
- 修复预览内搜索（`_search_within_preview`）和命中跳转（`_goto_*_preview_hit`）场景下 `find_sheet_matches` + `read_sheet_preview` 先后各打开一次文件的问题，改为一次 `read_sheet_with_hits` 调用

### Changed
- CI 触发范围调整：移除 `hotfix/*` 和 `feature/*` 分支的 push 触发，增加 `pull_request` 到 `master` 的触发，确立 master 为发布分支

## [1.4.0] - 2026-06-08

## What's Changed

### Fixed
- 修复深度索引完成后仍显示"0 已深度索引 / 全部待补全"的严重 bug — `_extract` 中 sheet id 与 cell_text 参数顺序颠倒，导致 UPDATE 从未命中任何行
- 修复重新扫描会清空已完成的深度索引数据 — upsert 更新已有文件时 DELETE sheet 后重新 INSERT 时 cell_text 写死为 NULL
- 修复 `add_file` 方法存在同样的 cell_text 丢失隐患
- 修复深度索引过程中文件提取失败时静默吞错 — 错误仅 print 到控制台，GUI 无任何提示
- 修复深度索引完成后耗时信息不显示 — `_on_deep_index_complete` 未使用 `_pending_status_prefix` 机制，耗时被后续搜索结果刷新覆盖
- 修复未选择扫描目录时点击「重新扫描」静默无反应
- 修复 QMessageBox 静态方法在 macOS 上不显示图标 — 改用 `QApplication.windowIcon()` 显式设置应用图标
- 修复 `alias_parser.py` 中不可达死代码
- 修复 `update_sheet_cell_texts_batch` 类型标注与实际参数顺序矛盾
- 修复非 macOS 平台首行文本右偏移的问题
- 修复深度索引处理超大 xlsx 文件时内存溢出 — 超过 200MB 的文件自动跳过并提示用户
- 修复 `extract_cell_texts` 中 `wb.close()` 异常时未执行导致的内存泄漏

### Improved
- **扫描性能大幅优化**：
  - sheet 名提取从 ET.parse DOM 解析改为编译正则，快 2-3x
  - 进度上报改为每 16 个文件一次，减少 GUI 信号开销
  - 按目录路径排序提交任务，改善磁盘局部性和 SSD 预读命中率
  - `zf.read()` 替代 `zf.open() + f.read()`，减少 zip 包装层开销
- `QThread.terminate()` 改为协作式取消（`_cancelled` 标志），避免强制终止线程可能导致 SQLite 连接泄漏
- 深度索引并发数从 4 降到 2，降低多文件同时加载的峰值内存
- 深度索引失败时收集错误并汇总弹窗提示用户
- 清理死代码：移除未使用的 `scan_file` / `scan_directory` 旧版同步扫描方法及多余 import
- 扫描进度栏与深度索引进度栏统一

## [1.3.1] - 2026-06-03

## What's Changed

### Fixed
- 修复 **Sheet 别名映射** 配置格式不统一的问题，确立标准格式 `英文配置名 子表名1 子表名2 ...`，同时兼容旧 `call do_conv.bat ...` 格式

## [1.3.0] - 2026-06-03

## What's Changed

### Added
- 新增 **命中单元格定位**，执行单元格搜索后，选中结果会自动跳到首个命中附近的预览窗口
- 新增 **预览高亮**，当前预览窗口内的命中单元格会高亮显示，并区分当前选中的命中位置
- 新增 **预览内搜索**，可在当前 sheet 内继续搜索，并支持「上一个 / 下一个」命中跳转
- 新增 **索引状态提示**，界面会显示文件数、子表数、已深度索引数和待补全数

### Improved
- 预览读取从固定左上角改为支持任意窗口起点，方便围绕命中区域展示内容
- 预览标题会显示命中数量与当前定位坐标，减少反复打开 Excel 确认位置的成本
- 搜不到结果时，状态栏会根据当前索引状态给出更具体的原因解释和操作建议
- 预览列宽分配优化为兼顾内容可读性和整体等宽铺满

## [1.2.1] - 2026-05-24

## What's Changed

### Added
- 新增 **Sheet 别名映射**功能，支持导入 bat/txt 映射脚本，可通过英文配置名搜索中文子表

### Fixed
- 修复 macOS 顶部菜单栏显示 "Python" 而非应用名的问题，通过 ObjC 运行时直接修改原生 NSMenu 标题
- 修复 macOS 统一标题栏模式下窗口标题文字与控件重叠的问题

### Improved
- macOS 标题栏改为透明统一样式（类似微信 macOS 端），内容区延伸至标题栏区域，视觉上融为一体
- 顶部工具栏支持拖拽移动窗口和双击最大化/恢复

## [1.2.0] - 2026-05-20

## What's Changed

### Added
- 新增**单元格内容搜索**，支持搜索所有 sheet 内的实际数据值（需先点击「深度索引」提取内容）
- 新增**深度索引**功能，后台提取所有 sheet 的单元格文本并入库，进度条实时反馈
- 新增**Sheet 预览面板**，选中搜索结果即时显示前 20 行数据，无需打开 Excel
- 新增**预览面板折叠/展开**，支持点击按钮或 `Ctrl+`` 快捷键切换
- 新增 **.xls 格式支持**，除 .xlsx/.xlsm 外也支持旧版 Excel 格式
- 新增**应用图标**，Windows/macOS/Linux 三平台适配

### Improved
- 搜索栏重组为两行布局，新增「单元格」输入框，搜索入口更清晰
- 结果区域改为可拖动分隔条，上方结果树 + 下方预览面板
- 搜索历史支持单元格搜索条件持久化
- macOS .app 打包修复，图标正确嵌入 Bundle

## [1.1.0] - 2026-05-14

## What's Changed

### Added
- 新增搜索匹配模式切换，支持模糊匹配、前缀匹配和精确匹配
- 新增搜索结果视图切换，支持分组视图和列表视图
- 新增搜索结果排序，支持按文件名和命中子表数排序
- 新增搜索结果统计显示，在状态栏展示当前文件数和子表命中数
- 新增最近搜索历史，支持恢复最近使用过的搜索条件组合
- 新增搜索结果导出功能，可将当前结果导出为 CSV 文件

### Improved
- 搜索结果改为按文件聚合后统一处理，文件级结果展示更清晰
- 启动时会恢复上次使用的扫描目录
- 启动时会恢复上次使用的匹配模式、排序方式和结果视图
- 目录显示补充完整路径提示，便于查看截断后的实际路径

### Changed
  - 统一数据库路径为 ~/.local/XlsxSearcher/index.db（所有平台）

## [1.0.0] - 2026-04-02

## What's Changed

### Added
- 新增扫描目录功能，支持递归扫描 xlsx/xlsm 文件
- 新增子表名称搜索和文件名搜索
- 新增"打开文件"功能，使用系统默认程序打开
- 新增"定位文件"功能，在文件管理器中定位文件
- 新增"复制路径"功能
- 新增扫描耗时显示（支持毫秒/秒/分钟显示）
- 新增增量扫描，只更新有变化的文件

### Improved
- 使用 PyQt5 重构 GUI
- 优化扫描性能：
  - 使用 zipfile 直接读取 xlsx 内部结构，速度提升 5-10 倍
  - 支持多线程并发扫描（默认 8 线程）
- 优化跨平台兼容性

### Fixed
- 修复 Windows 端定位文件功能失效的问题
- 修复数据库写入 bug（INSERT OR REPLACE 后 lastrowid 错误）
