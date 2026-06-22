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
