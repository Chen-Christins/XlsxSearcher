"""Desktop shell for the TypeScript UI using PyWebView."""
import multiprocessing
import os
import sys
import webbrowser

from web.server import AppState, create_server


def _choose_file_dialog(window, dialog_type, file_types=None):
    try:
        import webview

        kwargs = {}
        if file_types:
            kwargs["file_types"] = file_types
        result = window.create_file_dialog(dialog_type, **kwargs)
        if isinstance(result, tuple):
            return result[0] if result else None
        return result or None
    except Exception:
        return None


class DesktopApi:
    """Methods exposed to the frontend for native desktop operations."""

    def __init__(self):
        self.window = None
        self.url = ""

    def open_web_ui(self):
        webbrowser.open(self.url)
        return True

    def choose_directory(self):
        import webview

        return _choose_file_dialog(self.window, webview.FOLDER_DIALOG)

    def choose_alias_file(self):
        import webview

        return _choose_file_dialog(
            self.window,
            webview.OPEN_DIALOG,
            file_types=("Text files (*.bat;*.txt;*.cmd)",),
        )

    def choose_export_file(self):
        import webview

        return _choose_file_dialog(
            self.window,
            webview.SAVE_DIALOG,
            file_types=("CSV files (*.csv)",),
        )


def run_desktop():
    multiprocessing.freeze_support()
    state = AppState()
    server, port = create_server(state)
    url = f"http://127.0.0.1:{port}"
    print(f"WebUI 已启动: {url}")

    try:
        import webview
    except ImportError:
        print("未安装 pywebview，请先执行: pip install pywebview")
        print(f"开发预览可打开: {url}")
        return

    api = DesktopApi()
    api.url = url
    window = webview.create_window(
        "XlsxSearcher",
        url,
        js_api=api,
        width=1280,
        height=860,
        min_size=(1000, 700),
    )
    api.window = window
    webview.start()


if __name__ == "__main__":
    run_desktop()
