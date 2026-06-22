"""Benchmark deep indexing subprocess scheduling.

This script creates a temporary set of xlsx files, then compares:
1. serial subprocess extraction, matching the previous deep index scheduler
2. limited parallel subprocess extraction, matching the current scheduler shape

Run from the repo root:
    python benchmarks/deep_index_benchmark.py
"""
import argparse
import os
import shutil
import sys
import tempfile
import time
from multiprocessing import Pipe, freeze_support, get_context
from multiprocessing.connection import wait
from pathlib import Path

from openpyxl import Workbook

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from core.deep_index_worker import extract_file_cell_texts


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


def build_dataset(root: Path, files: int, sheets: int, rows: int, cols: int):
    jobs = []
    for file_index in range(files):
        filepath = root / f"bench_{file_index + 1:03d}.xlsx"
        create_workbook(filepath, sheets, rows, cols)
        jobs.append({
            "filepath": str(filepath),
            "sheet_names": [f"Sheet{i + 1}" for i in range(sheets)],
        })
    return jobs


def run_one_process(ctx, job, timeout_seconds: int):
    parent_conn, child_conn = Pipe(duplex=False)
    process = ctx.Process(
        target=extract_file_cell_texts,
        args=(job["filepath"], job["sheet_names"], child_conn, False),
    )
    process.start()
    child_conn.close()
    try:
        if parent_conn.poll(timeout_seconds):
            result = parent_conn.recv()
        else:
            process.terminate()
            process.join(5)
            if process.is_alive():
                process.kill()
                process.join(5)
            return False, f"Timed out after {timeout_seconds} seconds"

        process.join(5)
        if process.is_alive():
            process.terminate()
            process.join(5)

        if process.exitcode not in (0, None) and result.get("ok"):
            return False, f"Child process exited with code {process.exitcode}"
        if not result.get("ok"):
            return False, result.get("error", "unknown child process error")
        return True, result.get("texts") or []
    finally:
        parent_conn.close()
        if process.is_alive():
            process.kill()
            process.join(5)


def benchmark_serial(ctx, jobs, timeout_seconds: int):
    started = time.perf_counter()
    failures = []
    sheet_count = 0
    char_count = 0
    for job in jobs:
        ok, payload = run_one_process(ctx, job, timeout_seconds)
        if ok:
            texts = payload
            sheet_count += len(texts)
            char_count += sum(len(text) for text in texts)
        else:
            failures.append(f"{os.path.basename(job['filepath'])}: {payload}")
    return {
        "seconds": time.perf_counter() - started,
        "failures": failures,
        "sheet_count": sheet_count,
        "char_count": char_count,
    }


def benchmark_parallel(ctx, jobs, workers: int, timeout_seconds: int):
    started = time.perf_counter()
    failures = []
    sheet_count = 0
    char_count = 0
    pending = iter(jobs)
    active = {}

    def start_next():
        try:
            job = next(pending)
        except StopIteration:
            return False

        parent_conn, child_conn = Pipe(duplex=False)
        process = ctx.Process(
            target=extract_file_cell_texts,
            args=(job["filepath"], job["sheet_names"], child_conn, False),
        )
        process.start()
        child_conn.close()
        active[parent_conn] = {
            "process": process,
            "job": job,
            "started_at": time.perf_counter(),
        }
        return True

    def finish(conn, result):
        nonlocal sheet_count, char_count
        state = active.pop(conn)
        process = state["process"]
        job = state["job"]
        try:
            process.join(5)
            if process.is_alive():
                process.terminate()
                process.join(5)

            if process.exitcode not in (0, None) and result.get("ok"):
                result = {
                    "ok": False,
                    "error": f"Child process exited with code {process.exitcode}",
                }

            if result.get("ok"):
                texts = result.get("texts") or []
                sheet_count += len(texts)
                char_count += sum(len(text) for text in texts)
            else:
                failures.append(
                    f"{os.path.basename(job['filepath'])}: "
                    f"{result.get('error', 'unknown child process error')}"
                )
        finally:
            conn.close()
            if process.is_alive():
                process.kill()
                process.join(5)

    for _ in range(min(workers, len(jobs))):
        start_next()

    while active:
        now = time.perf_counter()
        timed_out = [
            conn for conn, state in active.items()
            if now - state["started_at"] >= timeout_seconds
        ]
        for conn in timed_out:
            process = active[conn]["process"]
            process.terminate()
            process.join(5)
            if process.is_alive():
                process.kill()
                process.join(5)
            finish(conn, {
                "ok": False,
                "error": f"Timed out after {timeout_seconds} seconds",
            })
            start_next()

        if not active:
            break

        for conn in wait(list(active.keys()), timeout=0.5):
            try:
                result = conn.recv()
            except EOFError:
                result = {
                    "ok": False,
                    "error": "Child process exited without returning a result",
                }
            finish(conn, result)
            start_next()

    return {
        "seconds": time.perf_counter() - started,
        "failures": failures,
        "sheet_count": sheet_count,
        "char_count": char_count,
    }


def print_result(name: str, result: dict) -> None:
    print(
        f"{name}: {result['seconds']:.2f}s, "
        f"{result['sheet_count']} sheets, "
        f"{result['char_count']:,} chars, "
        f"{len(result['failures'])} failures"
    )
    for failure in result["failures"][:5]:
        print(f"  - {failure}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--files", type=int, default=12)
    parser.add_argument("--sheets", type=int, default=4)
    parser.add_argument("--rows", type=int, default=1500)
    parser.add_argument("--cols", type=int, default=12)
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument(
        "--worker-sweep",
        default="",
        help="Comma-separated worker counts to compare, for example: 1,2,3,4.",
    )
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument(
        "--keep-data",
        action="store_true",
        help="Keep the generated temporary dataset and print its path.",
    )
    args = parser.parse_args()

    ctx = get_context("spawn")
    root = Path(tempfile.mkdtemp(prefix="xlsxsearcher_deep_index_bench_"))
    try:
        print(
            "Generating dataset: "
            f"{args.files} files x {args.sheets} sheets x "
            f"{args.rows} rows x {args.cols} cols"
        )
        jobs = build_dataset(root, args.files, args.sheets, args.rows, args.cols)

        serial = benchmark_serial(ctx, jobs, args.timeout)

        print_result("serial", serial)
        if args.worker_sweep:
            worker_counts = [
                int(value.strip())
                for value in args.worker_sweep.split(",")
                if value.strip()
            ]
        else:
            worker_counts = [args.workers]

        for workers in worker_counts:
            parallel = benchmark_parallel(ctx, jobs, workers, args.timeout)
            print_result(f"parallel({workers})", parallel)
            if parallel["seconds"] > 0:
                speedup = serial["seconds"] / parallel["seconds"]
                saved = serial["seconds"] - parallel["seconds"]
                print(f"speedup({workers}): {speedup:.2f}x, saved: {saved:.2f}s")

        if args.keep_data:
            print(f"dataset: {root}")
    finally:
        if not args.keep_data:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    freeze_support()
    main()
