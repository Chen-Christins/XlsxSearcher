"""XlsxSearcher - Excel子表搜索工具"""
import multiprocessing
import sys

def main():
    multiprocessing.freeze_support()
    from gui.app import run_app

    run_app()

if __name__ == '__main__':
    main()
