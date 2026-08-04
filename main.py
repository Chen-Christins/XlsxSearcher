"""XlsxSearcher - Excel子表搜索工具"""
import multiprocessing

def main():
    multiprocessing.freeze_support()
    from webui.desktop import run_desktop

    run_desktop()

if __name__ == '__main__':
    main()
