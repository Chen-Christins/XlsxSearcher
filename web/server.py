"""Local HTTP server used by the web-tech desktop UI.

The UI layer is a TypeScript single-page app. This server keeps all indexing,
scanning and search logic in the existing Python ``core`` modules.
"""
import json
import logging
import mimetypes
import os
import threading
from collections import defaultdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import yaml

from core.alias_parser import parse_sheet_alias_file, parse_sheet_alias_text
from core.indexer import IndexManager
from core.scanner import XlsxScanner
from core.searcher import Searcher
from utils.file_utils import copy_to_clipboard, open_file, open_in_explorer

ROOT = Path(__file__).resolve().parents[1]
STATIC_DIR = ROOT / "webui" / "dist"

with open(ROOT / "app.yml", encoding="utf-8") as _config_file:
    _CONFIG = yaml.safe_load(_config_file) or {}
APP_VERSION = _CONFIG.get("app", {}).get("version", "0.0.0")
_DATA_DIR = os.path.normpath(os.path.expanduser(_CONFIG.get("app", {}).get(
    "data_dir", "~/.local/XlsxSearcher"
)))
os.makedirs(_DATA_DIR, exist_ok=True)
STATE_FILE = os.path.join(_DATA_DIR, "webui_state.json")


class ApiError(Exception):
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


class AppState:
    """Shared application state for the local HTTP API."""

    def __init__(self):
        self.index_manager = IndexManager()
        self.scanner = XlsxScanner(use_calamine=True)
        self.searcher = Searcher(self.index_manager)
        self._lock = threading.Lock()
        self._directory = self._load_directory()
        self._job = {
            "running": False,
            "kind": "",
            "current": 0,
            "total": 0,
            "message": "",
            "result": None,
            "error": None,
        }

    def _load_directory(self):
        try:
            with open(STATE_FILE, encoding="utf-8") as state_file:
                value = json.load(state_file).get("directory", "")
            if value and os.path.isdir(value):
                return value
        except (OSError, ValueError):
            pass
        return ""

    def _save_directory(self):
        try:
            with open(STATE_FILE, "w", encoding="utf-8") as state_file:
                json.dump({"directory": self._directory}, state_file, ensure_ascii=False)
        except OSError:
            pass

    def get_state(self):
        with self._lock:
            job = dict(self._job)
        return {
            "version": APP_VERSION,
            "directory": self._directory,
            "index": self.index_manager.get_index_status(),
            "aliases": self.index_manager.get_sheet_alias_stats(),
            "job": job,
        }

    def search(self, params):
        sheet_keyword = params.get("sheet", "").strip()
        filename_keyword = params.get("filename", "").strip()
        cell_keyword = params.get("cell", "").strip()
        match_mode = params.get("match_mode", "fuzzy")
        sort_mode = params.get("sort_mode", "filename_asc")

        if not any((sheet_keyword, filename_keyword, cell_keyword)):
            results = self.index_manager.get_all_files_with_sheets(sort_mode=sort_mode)
        else:
            results = self.searcher.search(
                sheet_keyword,
                filename_keyword,
                cell_keyword,
                match_mode,
                sort_mode,
            )
        return {
            "results": results,
            "total_files": len(results),
            "total_sheets": sum(result.get("sheet_count", 0) for result in results),
        }

    def preview(self, params):
        filepath = params.get("filepath", "")
        sheet_name = params.get("sheet_name", "")
        if not filepath or not sheet_name:
            raise ApiError(400, "filepath 和 sheet_name 不能为空")
        if not os.path.exists(filepath):
            raise ApiError(404, "文件不存在或已被移动")

        keyword = params.get("keyword", "") or None
        match_mode = params.get("match_mode", "fuzzy")
        start_row = self._int_param(params, "start_row")
        start_col = self._int_param(params, "start_col")

        hits, data, header_row = self.scanner.read_sheet_with_hits(
            filepath,
            sheet_name,
            keyword=keyword,
            match_mode=match_mode,
            max_hits=200,
            preview_rows=20,
            preview_cols=50,
            start_row=start_row,
            start_col=start_col,
        )

        if start_row is None:
            if hits:
                start_row = max(int(hits[0]["row"]) - 3, 1)
                start_col = max(int(hits[0]["col"]) - 2, 1)
            else:
                start_row, start_col = 2, 1

        return {
            "hits": hits,
            "data": data,
            "header_row": header_row,
            "start_row": start_row,
            "start_col": start_col,
        }

    def start_scan(self, directory):
        directory = os.path.abspath(os.path.expanduser(directory or ""))
        if not os.path.isdir(directory):
            raise ApiError(400, "目录不存在或不可读")

        with self._lock:
            if self._job["running"]:
                raise ApiError(409, "已有任务正在执行")
            self._directory = directory
            self._save_directory()
            self._job = {
                "running": True,
                "kind": "scan",
                "current": 0,
                "total": 0,
                "message": "正在扫描...",
                "result": None,
                "error": None,
            }

        threading.Thread(target=self._scan_worker, args=(directory,), daemon=True).start()
        return {"ok": True}

    def start_deep_index(self):
        with self._lock:
            if self._job["running"]:
                raise ApiError(409, "已有任务正在执行")
            self._job = {
                "running": True,
                "kind": "deep_index",
                "current": 0,
                "total": 0,
                "message": "正在提取单元格内容...",
                "result": None,
                "error": None,
            }

        threading.Thread(target=self._deep_index_worker, daemon=True).start()
        return {"ok": True}

    def clear_index(self):
        with self._lock:
            if self._job["running"]:
                raise ApiError(409, "任务执行中，请稍后再清空索引")
        self.index_manager.clear_index()
        return {"ok": True}

    def import_aliases(self, source_name, content):
        mappings = parse_sheet_alias_text(content)
        if not mappings:
            raise ApiError(400, "未解析到有效映射")
        imported = self.index_manager.replace_sheet_aliases(source_name, mappings)
        return {
            "ok": True,
            "imported": imported,
            "mapping_count": len(mappings),
        }

    def import_aliases_from_file(self, path):
        if not os.path.isfile(path):
            raise ApiError(400, "映射文件不存在")
        mappings = parse_sheet_alias_file(path)
        if not mappings:
            raise ApiError(400, "未解析到有效映射")
        imported = self.index_manager.replace_sheet_aliases(
            os.path.abspath(path), mappings
        )
        return {
            "ok": True,
            "imported": imported,
            "mapping_count": len(mappings),
        }

    def export_results(self, path, params):
        search_response = self.search(params)
        results = search_response["results"]
        directory = os.path.dirname(path)
        if directory:
            os.makedirs(directory, exist_ok=True)
        import csv

        with open(path, "w", newline="", encoding="utf-8-sig") as csv_file:
            writer = csv.writer(csv_file)
            writer.writerow(["文件名", "子表名称", "别名", "文件路径", "命中子表数"])
            for result in results:
                sheet_names = result.get("sheet_names", [])
                if sheet_names:
                    for sheet_name in sheet_names:
                        writer.writerow([
                            result["filename"],
                            sheet_name,
                            ", ".join(result.get("sheet_aliases", {}).get(sheet_name, [])),
                            result["filepath"],
                            result.get("sheet_count", 0),
                        ])
                else:
                    writer.writerow([
                        result["filename"],
                        "",
                        "",
                        result["filepath"],
                        result.get("sheet_count", 0),
                    ])
        return {"ok": True, "exported": len(results), "path": path}

    def run_action(self, action, filepath):
        if not filepath:
            raise ApiError(400, "缺少文件路径")
        if action == "open":
            open_file(filepath)
        elif action == "locate":
            open_in_explorer(filepath)
        elif action == "copy":
            if not copy_to_clipboard(filepath):
                raise ApiError(500, "复制到剪贴板失败")
        else:
            raise ApiError(400, f"未知操作: {action}")
        return {"ok": True}

    def _scan_worker(self, directory):
        try:
            result = self.scanner.scan_directory_incremental(
                directory,
                self.index_manager,
                progress_callback=self._set_progress,
            )
            added, updated, deleted = result
            self._finish_job(
                result={
                    "added": added,
                    "updated": updated,
                    "deleted": deleted,
                },
                message=f"扫描完成：新增 {added}，更新 {updated}，删除 {deleted}",
            )
        except Exception as exc:
            logging.exception("scan failed")
            self._finish_job(error=str(exc))

    def _deep_index_worker(self):
        from multiprocessing import get_context

        from core.deep_index_worker import extract_file_cell_texts_pool

        try:
            pending = self.index_manager.get_sheets_without_cell_text()
            total = len(pending)
            if total == 0:
                self._finish_job(result={"processed": 0}, message="深度索引已是最新")
                return

            by_file = defaultdict(list)
            for entry in pending:
                by_file[entry["filepath"]].append(entry)

            max_workers = min(
                4,
                max(1, (os.cpu_count() or 2) - 1),
                len(by_file),
            )
            file_args = []
            file_meta = {}
            for filepath, entries in by_file.items():
                sheet_ids = [entry["sheet_id"] for entry in entries]
                file_args.append((filepath, [entry["sheet_name"] for entry in entries], True))
                file_meta[filepath] = {"sheet_ids": sheet_ids, "count": len(entries)}

            processed = 0
            errors = []
            ctx = get_context("spawn")
            with ctx.Pool(max_workers) as pool:
                for result in pool.imap_unordered(extract_file_cell_texts_pool, file_args):
                    filepath = result["filepath"]
                    meta = file_meta[filepath]
                    if result.get("ok"):
                        updates = [
                            (text, sheet_id)
                            for text, sheet_id in zip(
                                result.get("texts") or [], meta["sheet_ids"]
                            )
                        ]
                        if updates:
                            self.index_manager.update_sheet_cell_texts_batch(updates)
                    else:
                        errors.append(f"{os.path.basename(filepath)}: {result.get('error')}")
                    processed += meta["count"]
                    self._set_progress(processed, total)

            message = f"深度索引完成：已处理 {processed}/{total} 个子表"
            if errors:
                message += f"；{len(errors)} 个文件失败，详见日志"
            self._finish_job(result={"processed": processed, "errors": len(errors)}, message=message)
        except Exception as exc:
            logging.exception("deep index failed")
            self._finish_job(error=str(exc))

    def _set_progress(self, current, total):
        with self._lock:
            self._job["current"] = current
            self._job["total"] = total

    def _finish_job(self, result=None, message="", error=None):
        with self._lock:
            self._job.update(
                running=False,
                result=result,
                message=message,
                error=error,
            )

    @staticmethod
    def _int_param(params, name):
        value = params.get(name)
        if value in (None, ""):
            return None
        try:
            return max(int(value), 1)
        except (TypeError, ValueError):
            return None


class ApiHandler(BaseHTTPRequestHandler):
    server_version = "XlsxSearcherWeb/1.0"

    def do_GET(self):
        try:
            path = urlsplit(self.path).path
            if path.startswith("/api/"):
                self._handle_api_get(path)
            else:
                self._serve_static(path)
        except ApiError as exc:
            self._send_json({"ok": False, "error": exc.message}, exc.status)
        except Exception as exc:
            logging.exception("GET %s failed", self.path)
            self._send_json({"ok": False, "error": str(exc)}, 500)

    def do_POST(self):
        try:
            path = urlsplit(self.path).path
            body = self._read_json()
            if path == "/api/scan":
                payload = self.server.state.start_scan(body.get("directory", ""))
            elif path == "/api/deep-index":
                payload = self.server.state.start_deep_index()
            elif path == "/api/clear-index":
                payload = self.server.state.clear_index()
            elif path == "/api/action":
                payload = self.server.state.run_action(
                    body.get("action", ""),
                    body.get("filepath", ""),
                )
            elif path == "/api/import-aliases":
                payload = self.server.state.import_aliases(
                    body.get("source_name", "imported.txt"),
                    body.get("content", ""),
                )
            elif path == "/api/import-aliases-file":
                payload = self.server.state.import_aliases_from_file(
                    body.get("path", "")
                )
            elif path == "/api/export":
                payload = self.server.state.export_results(
                    body.get("path", ""),
                    {
                        "sheet": body.get("sheet", ""),
                        "filename": body.get("filename", ""),
                        "cell": body.get("cell", ""),
                        "match_mode": body.get("match_mode", "fuzzy"),
                        "sort_mode": body.get("sort_mode", "filename_asc"),
                    },
                )
            else:
                raise ApiError(404, "接口不存在")
            self._send_json({"ok": True, **payload})
        except ApiError as exc:
            self._send_json({"ok": False, "error": exc.message}, exc.status)
        except Exception as exc:
            logging.exception("POST %s failed", self.path)
            self._send_json({"ok": False, "error": str(exc)}, 500)

    def _handle_api_get(self, path):
        params = {
            key: values[0]
            for key, values in parse_qs(urlsplit(self.path).query).items()
        }
        if path == "/api/state":
            payload = self.server.state.get_state()
        elif path == "/api/search":
            payload = self.server.state.search(params)
        elif path == "/api/preview":
            payload = self.server.state.preview(params)
        elif path == "/api/health":
            payload = {"ok": True}
        else:
            raise ApiError(404, "接口不存在")
        self._send_json({"ok": True, **payload})

    def _read_json(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        if length <= 0:
            return {}
        if length > 8 * 1024 * 1024:
            raise ApiError(413, "请求体过大")
        raw = self.rfile.read(length)
        try:
            return json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as exc:
            raise ApiError(400, "请求体不是有效 JSON") from exc

    def _serve_static(self, path):
        if path in ("", "/"):
            relative = "index.html"
        else:
            relative = path.lstrip("/")
        full_path = os.path.realpath(os.path.join(STATIC_DIR, relative))
        static_root = os.path.realpath(STATIC_DIR)
        if full_path != static_root and not full_path.startswith(static_root + os.sep):
            raise ApiError(403, "禁止访问")
        if not os.path.isfile(full_path):
            raise ApiError(404, "文件不存在")

        content_type, _ = mimetypes.guess_type(full_path)
        with open(full_path, "rb") as static_file:
            data = static_file.read()
        self.send_response(200)
        self.send_header("Content-Type", content_type or "application/octet-stream")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def _send_json(self, payload, status=200):
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        logging.info("webui %s", fmt % args)


def create_server(state=None):
    """Start the local API/static server and return (server, port)."""
    if state is None:
        state = AppState()
    server = ThreadingHTTPServer(("127.0.0.1", 0), ApiHandler)
    server.daemon_threads = True
    server.state = state
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, port
