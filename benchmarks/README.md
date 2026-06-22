# XlsxSearcher 性能对比测试

本目录用于验证三项优化的正确性并量化收益：

1. **预览异步化 + 窗口化读取**（`core/scanner.py`）：`read_sheet_with_hits` 改为单遍流式，
   只保留有界滚动窗口，不再整表载入内存。
2. **FTS5 替代 `cell_text LIKE '%kw%'`**（`core/indexer.py`）：单元格搜索走 trigram 倒排索引。
3. **连接复用 + upsert 跳过未变 sheets**（`core/indexer.py`）：每线程长连接；sheet 名未变时跳过重建。

## 运行

```bash
pip install openpyxl python-calamine xlrd
python benchmarks/bench.py
```

脚本会：
- 生成临时 xlsx 测试数据（中等规模 + 一张大表）；
- 对每项优化做 **正确性断言**（新实现与旧实现结果一致）；
- 输出 **性能对比表**（FTS vs LIKE、流式窗口 vs 全量载入、二次扫描跳过）。

脚本结束自动清理临时文件，不污染用户索引库（使用独立临时 db）。
