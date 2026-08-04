"""Standalone browser mode: run the same UI in the default web browser."""
import time

from web.server import AppState, create_server


def run_web():
    state = AppState()
    server, port = create_server(state)
    url = f"http://127.0.0.1:{port}"
    print(f"Web 界面已启动: {url}")

    import webbrowser

    webbrowser.open(url)
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        server.shutdown()


if __name__ == "__main__":
    run_web()
