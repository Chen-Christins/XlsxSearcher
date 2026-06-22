# Benchmarks 性能测试

本目录存放本地性能测试脚本。脚本会生成临时 XLSX 文件，并使用临时 SQLite 数据库，不会读写用户真实的 `~/.local/XlsxSearcher/index.db`。

先安装运行依赖：

```bash
pip install -r requirements.txt
```

## 综合性能测试

```bash
python benchmarks/bench.py
```

这是较早的综合 benchmark，用于覆盖预览窗口读取、单元格搜索、增量扫描等核心路径的正确性和性能对比。

## 深度索引性能测试

```bash
python benchmarks/deep_index_benchmark.py
```

用于对比深度索引的子进程调度性能：

- 串行子进程提取
- 有限并发子进程提取

常用参数：

```bash
python benchmarks/deep_index_benchmark.py --files 12 --sheets 4 --rows 1500 --cols 12
python benchmarks/deep_index_benchmark.py --worker-sweep 1,2,3,4
python benchmarks/deep_index_benchmark.py --workers 4 --timeout 180
```

输出内容包括总耗时、处理 sheet 数、提取字符数、失败数，以及相对串行提取的加速比。

## 扫描性能测试

```bash
python benchmarks/scan_benchmark.py
```

用于按阶段统计普通目录扫描路径的耗时：

- 目录遍历和文件状态读取
- 已有索引快照查询
- 新增/变更文件筛选
- 从 workbook 元数据提取 sheet 名
- SQLite 批量写入

常用参数：

```bash
python benchmarks/scan_benchmark.py --files 200 --sheets 4 --rows 20 --cols 8
python benchmarks/scan_benchmark.py --workers 8
python benchmarks/scan_benchmark.py --skip-warm-scan
```

默认会跑首次扫描和第二次无变更扫描，并输出无变更扫描相对首次扫描的耗时比例。

## 保留生成数据

较新的 benchmark 脚本支持 `--keep-data`，用于调试时保留生成的临时文件：

```bash
python benchmarks/deep_index_benchmark.py --keep-data
python benchmarks/scan_benchmark.py --keep-data
```

不加 `--keep-data` 时，脚本结束会自动清理临时数据。

