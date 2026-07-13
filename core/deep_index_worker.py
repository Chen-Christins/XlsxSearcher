"""Subprocess entrypoints for deep cell indexing."""
import traceback
from multiprocessing.connection import Connection
from typing import List


def extract_file_cell_texts(
    filepath: str,
    sheet_names: List[str],
    conn: Connection,
    use_calamine: bool = True,
):
    """Extract one workbook in a child process and send a small result payload."""
    try:
        from core.scanner import XlsxScanner

        scanner = XlsxScanner(max_workers=1, use_calamine=use_calamine)
        texts = scanner.extract_cell_texts(filepath, sheet_names)
        conn.send({"ok": True, "texts": texts})
    except BaseException as exc:
        conn.send({
            "ok": False,
            "error": f"{type(exc).__name__}: {exc}",
            "traceback": traceback.format_exc(),
        })
    finally:
        conn.close()


def extract_file_cell_texts_pool(args):
    """Pool-compatible worker: extract cell texts and return result dict directly.

    Avoids per-file process spawn overhead; processes are reused across files.
    Args: (filepath, sheet_names, use_calamine)
    Returns: dict with 'filepath', 'ok', 'texts' or 'error'/'traceback'
    """
    filepath, sheet_names, use_calamine = args
    try:
        from core.scanner import XlsxScanner

        scanner = XlsxScanner(max_workers=1, use_calamine=use_calamine)
        texts = scanner.extract_cell_texts(filepath, sheet_names)
        return {"ok": True, "filepath": filepath, "texts": texts}
    except BaseException as exc:
        return {
            "ok": False,
            "filepath": filepath,
            "error": f"{type(exc).__name__}: {exc}",
            "traceback": traceback.format_exc(),
        }
