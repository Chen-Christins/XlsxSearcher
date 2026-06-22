"""Benchmark the normal directory scan path.

The scan path is split into:
1. directory walk and mtime collection
2. indexed snapshot lookup
3. changed-file filtering
4. sheet-name extraction
5. SQLite batch write

Run from the repo root:
    python benchmarks/scan_benchmark.py
"""
import argparse
import os
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from openpyxl import Workbook

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from core.indexer import IndexManager
from core.scanner import XlsxScanner


def create_workbook(filepath: Path, sheets: int, rows: int, cols: int) -> None:
    wb = Workbook(write_only=True)
    for sheet_index in range(sheets):
        ws = wb.create_sheet(title=f"Sheet{sheet_index + 1}")
        for row_index in range(rows):
            ws.append([
                f"file={filepath.stem} sheet={sheet_index + 1} row={row_index + 1} col={col + 1}"
                for col in range(cols)
            ])
    wb.save(filepath)


def build_dataset(root: Path, files: int, sheets: int, rows: int, cols: int) -> None:
    for file_index in range(files):
        filepath = root / f"scan_{file_index + 1:04d}.xlsx"
        create_workbook(filepath, sheets, rows, cols)


def format_seconds(seconds: float) -> str:
    return f"{seconds:.4f}s"


def print_timing(name: str, seconds: float, total: float) -> None:
    share = (seconds / total * 100) if total > 0 else 0
    print(f"{name:24s} {format_seconds(seconds):>10s}  {share:6.2f}%")


def timed_scan(directory: Path, db_path: Path, max_workers: int) -> dict:
    scanner = XlsxScanner(max_workers=max_workers)
    index_manager = IndexManager(str(db_path))
    timings = {}

    total_started = time.perf_counter()

    started = time.perf_counter()
    all_files = {}
    for root, entry in scanner._walk_files(str(directory)):
        if not scanner.is_xlsx_file(entry.name):
            continue
        try:
            stat = entry.stat()
        except OSError:
            continue
        all_files[os.path.join(root, entry.name)] = stat.st_mtime
    timings["walk_and_stat"] = time.perf_counter() - started

    started = time.perf_counter()
    indexed = index_manager.get_all_files_indexed()
    timings["indexed_snapshot"] = time.perf_counter() - started

    started = time.perf_counter()
    files_to_process = []
    for filepath, mtime in all_files.items():
        existing = indexed.get(filepath)
        if existing is None or existing[1] != mtime:
            files_to_process.append((filepath, os.path.basename(filepath), mtime))
    files_to_process.sort(key=lambda item: item[0])
    timings["filter_changed"] = time.perf_counter() - started

    started = time.perf_counter()
    pending_updates = []
    if files_to_process:
        with ThreadPoolExecutor(max_workers=max_workers) as executor:
            futures = {
                executor.submit(scanner.get_sheet_names, filepath): (filepath, filename, mtime)
                for filepath, filename, mtime in files_to_process
            }
            for future in as_completed(futures):
                filepath, filename, mtime = futures[future]
                sheet_names = future.result()
                if sheet_names:
                    pending_updates.append((filename, filepath, mtime, sheet_names))
    timings["sheet_name_extract"] = time.perf_counter() - started

    started = time.perf_counter()
    added, updated = index_manager.upsert_files_batch(pending_updates, indexed)
    timings["sqlite_upsert"] = time.perf_counter() - started

    timings["total"] = time.perf_counter() - total_started
    return {
        "timings": timings,
        "file_count": len(all_files),
        "changed_count": len(files_to_process),
        "sheet_count": sum(len(update[3]) for update in pending_updates),
        "added": added,
        "updated": updated,
    }


def run_case(name: str, directory: Path, db_path: Path, max_workers: int) -> dict:
    result = timed_scan(directory, db_path, max_workers)
    timings = result["timings"]
    print(f"\n{name}")
    print(
        f"files={result['file_count']}, changed={result['changed_count']}, "
        f"sheets={result['sheet_count']}, added={result['added']}, updated={result['updated']}"
    )
    for key in [
        "walk_and_stat",
        "indexed_snapshot",
        "filter_changed",
        "sheet_name_extract",
        "sqlite_upsert",
    ]:
        print_timing(key, timings[key], timings["total"])
    print_timing("total", timings["total"], timings["total"])
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--files", type=int, default=200)
    parser.add_argument("--sheets", type=int, default=4)
    parser.add_argument("--rows", type=int, default=20)
    parser.add_argument("--cols", type=int, default=8)
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument(
        "--skip-warm-scan",
        action="store_true",
        help="Only run the first scan; by default a second no-change scan is also measured.",
    )
    parser.add_argument(
        "--keep-data",
        action="store_true",
        help="Keep the generated dataset and temporary SQLite DB.",
    )
    args = parser.parse_args()

    temp_dir = tempfile.TemporaryDirectory(prefix="xlsxsearcher_scan_bench_")
    root = Path(temp_dir.name)
    data_dir = root / "data"
    data_dir.mkdir()
    db_path = root / "scan_benchmark.db"

    try:
        print(
            "Generating dataset: "
            f"{args.files} files x {args.sheets} sheets x "
            f"{args.rows} rows x {args.cols} cols"
        )
        build_dataset(data_dir, args.files, args.sheets, args.rows, args.cols)

        first = run_case("first scan", data_dir, db_path, args.workers)
        if not args.skip_warm_scan:
            second = run_case("second scan, no file changes", data_dir, db_path, args.workers)
            if first["timings"]["total"] > 0:
                ratio = second["timings"]["total"] / first["timings"]["total"]
                print(f"\nno-change scan ratio: {ratio:.2%} of first scan")

        if args.keep_data:
            print(f"\ndataset: {data_dir}")
            print(f"database: {db_path}")
            temp_dir = None
    finally:
        if temp_dir is not None:
            temp_dir.cleanup()


if __name__ == "__main__":
    main()
