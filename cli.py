#!/usr/bin/env python3
"""XlsxSearcher CLI - 复用 core 层能力，不依赖 PyQt。

示例:
    python cli.py scan ./configs
    python cli.py search --sheet ItemConfig --format json
    python cli.py ask "新手礼包在哪个表"
"""
import argparse
import csv
import json
import multiprocessing
import os
import re
import sys
import time
import urllib.error
import urllib.request
from collections import defaultdict
from typing import Dict, List, Tuple

import yaml

from core.alias_parser import parse_sheet_alias_file
from core.deep_index_worker import extract_file_cell_texts_pool
from core.indexer import IndexManager
from core.scanner import XlsxScanner
from core.searcher import Searcher


APP_ROOT = os.path.dirname(os.path.abspath(__file__))
CONFIG_PATH = os.path.join(APP_ROOT, 'app.yml')
DEFAULT_DATA_DIR = os.path.expanduser('~/.local/XlsxSearcher')

_DEFAULT_MODEL = 'gpt-4o-mini'
_STOP_CHARS = set(
    '的了吗呢吧啊是在里有和与及或对你我他她它们这那哪个什么怎么多少'
    '是否可以请帮问一下当前目前找到查找搜索看看文件子表配置表里面对应'
    '答案给出需要知道想要请问'
)


class CliError(Exception):
    """CLI 可预期的错误，main 统一输出。"""


def _load_app_config() -> Dict:
    try:
        with open(CONFIG_PATH, encoding='utf-8') as f:
            cfg = yaml.safe_load(f) or {}
        return cfg.get('app', {})
    except Exception:
        return {}


def _make_manager(args) -> IndexManager:
    """创建 IndexManager，优先使用 --db，其次读取 app.yml 的 data_dir。"""
    db_path = getattr(args, 'db', None)
    if not db_path:
        data_dir = _load_app_config().get('data_dir') or DEFAULT_DATA_DIR
        data_dir = os.path.normpath(os.path.expanduser(data_dir))
        db_path = os.path.join(data_dir, 'index.db')
    db_dir = os.path.dirname(os.path.abspath(db_path))
    os.makedirs(db_dir, exist_ok=True)
    return IndexManager(db_path=db_path)


def _write_csv_rows(results, writer) -> None:
    writer.writerow(['文件名', '子表名称', '别名', '文件路径', '命中子表数'])
    for result in results:
        sheet_names = result.get('sheet_names') or ['']
        for sheet_name in sheet_names:
            aliases = ''
            if sheet_name:
                aliases = ', '.join(result.get('sheet_aliases', {}).get(sheet_name, []))
            writer.writerow([
                result['filename'],
                sheet_name,
                aliases,
                result['filepath'],
                result.get('sheet_count', 0),
            ])


def _write_csv_file(results, output_path: str) -> None:
    with open(output_path, 'w', newline='', encoding='utf-8-sig') as f:
        _write_csv_rows(results, csv.writer(f))


def _write_search_output(results, fmt: str, stream, export_path: str = None) -> None:
    if export_path:
        _write_csv_file(results, export_path)
        return

    if fmt == 'json':
        json.dump(results, stream, ensure_ascii=False, indent=2)
        stream.write('\n')
        return

    if fmt == 'csv':
        _write_csv_rows(results, csv.writer(stream))
        return

    for result in results:
        aliases_by_sheet = result.get('sheet_aliases', {})
        print(
            f"{result['filename']}  {result['filepath']}  "
            f"({result.get('sheet_count', 0)} 子表)",
            file=stream,
        )
        for sheet_name in result.get('sheet_names', []):
            aliases = ', '.join(aliases_by_sheet.get(sheet_name, []))
            if aliases:
                print(f"  {sheet_name}  ->  {aliases}", file=stream)
            else:
                print(f"  {sheet_name}", file=stream)


def _collect_search_results(args) -> List[Dict]:
    """按参数执行搜索；无过滤条件时列出全部文件。"""
    manager = _make_manager(args)
    if manager.get_stats()['file_count'] == 0:
        return []

    searcher = Searcher(manager)
    if not (args.sheet or args.filename or args.cell):
        results = manager.get_all_files_with_sheets(sort_mode=args.sort)
    else:
        results = searcher.search(
            args.sheet, args.filename, args.cell, args.match, args.sort
        )

    limit = getattr(args, 'limit', 0)
    if limit and limit > 0:
        results = results[:limit]
    return results


def _add_search_args(parser) -> None:
    parser.add_argument('--sheet', default='', help='子表名称或英文配置名')
    parser.add_argument('--filename', default='', help='文件名关键字')
    parser.add_argument('--cell', default='', help='单元格内容关键字')
    parser.add_argument(
        '--match', choices=['fuzzy', 'prefix', 'exact'], default='fuzzy',
        help='匹配模式（默认 fuzzy）',
    )
    parser.add_argument(
        '--sort',
        choices=['filename_asc', 'filename_desc', 'sheet_count_desc', 'sheet_count_asc'],
        default='filename_asc',
        help='排序方式',
    )
    parser.add_argument('--limit', type=int, default=0, help='最多输出多少条文件结果')


def cmd_scan(args) -> int:
    directory = os.path.abspath(args.directory)
    if not os.path.isdir(directory):
        raise CliError(f'目录不存在: {directory}')

    manager = _make_manager(args)
    scanner = XlsxScanner(use_calamine=True)
    start = time.time()

    def on_progress(current, total):
        if args.quiet or not sys.stderr.isatty() or total <= 0:
            return
        print(f'扫描进度: {current}/{total}', file=sys.stderr, end='\r')
        if current >= total:
            print(file=sys.stderr)

    added, updated, deleted = scanner.scan_directory_incremental(
        directory, manager, progress_callback=on_progress
    )
    duration = time.time() - start
    stats = manager.get_stats()
    print(f'索引完成: 新增 {added}, 更新 {updated}, 删除 {deleted}, 耗时 {duration:.1f}s')
    print(f'当前索引: {stats["file_count"]} 个文件 / {stats["sheet_count"]} 个子表')
    return 0


def cmd_deep_index(args) -> int:
    manager = _make_manager(args)
    pending = manager.get_sheets_without_cell_text()
    total = len(pending)
    if total == 0:
        print('深度索引已是最新: 没有待提取的子表')
        return 0

    by_file = defaultdict(list)
    for entry in pending:
        by_file[entry['filepath']].append(entry)

    cpu_count = os.cpu_count() or 2
    max_workers = args.processes if args.processes and args.processes > 0 else min(
        4, max(1, cpu_count - 1), len(by_file)
    )
    file_args = [(filepath, [e['sheet_name'] for e in entries], True)
                 for filepath, entries in by_file.items()]
    file_meta = {
        filepath: {'sheet_ids': [e['sheet_id'] for e in entries], 'count': len(entries)}
        for filepath, entries in by_file.items()
    }

    errors = []
    processed = 0
    start = time.time()
    ctx = multiprocessing.get_context('spawn')
    with ctx.Pool(max_workers) as pool:
        for result in pool.imap_unordered(extract_file_cell_texts_pool, file_args):
            filepath = result['filepath']
            meta = file_meta[filepath]
            if result.get('ok'):
                texts = result.get('texts') or []
                updates = [
                    (text, sheet_id)
                    for text, sheet_id in zip(texts, meta['sheet_ids'])
                ]
                if updates:
                    manager.update_sheet_cell_texts_batch(updates)
            else:
                error_text = result.get('error', 'unknown error')
                errors.append(f'{os.path.basename(filepath)}: {error_text}')

            processed += meta['count']
            if not args.quiet and sys.stderr.isatty():
                print(
                    f'深度索引: {processed}/{total}',
                    file=sys.stderr,
                    end='\r',
                )
                if processed >= total:
                    print(file=sys.stderr)

    duration = time.time() - start
    print(f'深度索引完成: 已处理 {processed}/{total}, 耗时 {duration:.1f}s')
    if errors:
        print(f'失败 {len(errors)} 个文件:', file=sys.stderr)
        for error in errors[:5]:
            print(f'  {error}', file=sys.stderr)
    return 0


def cmd_search(args) -> int:
    results = _collect_search_results(args)
    if args.export:
        _write_csv_file(results, args.export)
        print(f'已导出 {len(results)} 个文件到 {args.export}')
        return 0
    _write_search_output(results, args.format, sys.stdout)
    return 0


def cmd_export(args) -> int:
    results = _collect_search_results(args)
    _write_csv_file(results, args.output)
    print(f'已导出 {len(results)} 个文件到 {args.output}')
    return 0


def cmd_alias(args) -> int:
    manager = _make_manager(args)
    if args.alias_command == 'import':
        if not os.path.isfile(args.file):
            raise CliError(f'映射文件不存在: {args.file}')
        mappings = parse_sheet_alias_file(args.file)
        if not mappings:
            raise CliError('未在该文件中解析到有效映射')
        inserted = manager.replace_sheet_aliases(args.file, mappings)
        stats = manager.get_sheet_alias_stats()
        print(
            f'已导入 {inserted} 条映射，'
            f'当前共 {stats["alias_count"]} 个英文名 / {stats["mapping_count"]} 条映射'
        )
        return 0

    aliases = manager.get_all_sheet_aliases()
    if not aliases:
        print('暂无别名映射')
        return 0
    for sheet_name in sorted(aliases, key=str.lower):
        print(f'{sheet_name}: {", ".join(aliases[sheet_name])}')
    return 0


def cmd_stats(args) -> int:
    manager = _make_manager(args)
    index = manager.get_index_status()
    aliases = manager.get_sheet_alias_stats()
    if args.format == 'json':
        json.dump({'index': index, 'aliases': aliases}, sys.stdout, ensure_ascii=False, indent=2)
        sys.stdout.write('\n')
        return 0

    print(
        f'文件: {index["file_count"]} / '
        f'子表: {index["sheet_count"]} / '
        f'已深度索引: {index["indexed_cell_sheet_count"]} / '
        f'待补全: {index["pending_deep_index_count"]}'
    )
    print(
        f'别名: {aliases["alias_count"]} 个英文名 / '
        f'{aliases["mapping_count"]} 条映射'
    )
    return 0


def cmd_version(args) -> int:
    version = _load_app_config().get('version', '0.0.0')
    print(f'xlsxsearcher {version}')
    return 0


# ---- ask: 检索增强问答 ----


def _is_useful_chinese_gram(gram: str) -> bool:
    return any(char not in _STOP_CHARS for char in gram)


def _extract_search_terms(question: str) -> List[str]:
    """从自然语言问题中提取可检索的英文名与中文片段。"""
    terms = set()
    for match in re.finditer(r'["“]([^"”]+)["”]', question):
        quoted = match.group(1).strip()
        if quoted:
            terms.add(quoted)

    cleaned = re.sub(r'["“”]', ' ', question)
    for token in re.findall(r'[A-Za-z][A-Za-z0-9_]{2,}', cleaned):
        terms.add(token)

    for chunk in re.findall(r'[\u4e00-\u9fff]+', cleaned):
        if len(chunk) <= 6:
            if _is_useful_chinese_gram(chunk):
                terms.add(chunk)
            continue
        for size in (4, 3, 2):
            for index in range(0, len(chunk) - size + 1):
                gram = chunk[index:index + size]
                if _is_useful_chinese_gram(gram):
                    terms.add(gram)
            if len(terms) >= 24:
                break

    ordered = sorted(
        terms,
        key=lambda term: (not re.match(r'^[A-Za-z]', term), -len(term)),
    )
    return ordered[:20] or [question]


def _retrieve_candidates(manager: IndexManager, question: str, max_results: int):
    """用现有索引做召回：子表名命中权重高于单元格内容命中。"""
    searcher = Searcher(manager)
    scored = {}
    for term in _extract_search_terms(question):
        try:
            sheet_hits = searcher.search(sheet_keyword=term, match_mode='fuzzy')
        except Exception:
            sheet_hits = []
        for result in sheet_hits:
            for sheet_name in result.get('sheet_names', []):
                key = (result['filepath'], sheet_name)
                entry = scored.setdefault(key, {'score': 0, 'terms': set(), 'filename': result['filename']})
                entry['score'] += 3
                entry['terms'].add(term)

        try:
            cell_hits = searcher.search(cell_keyword=term, match_mode='fuzzy')
        except Exception:
            cell_hits = []
        for result in cell_hits:
            for sheet_name in result.get('sheet_names', []):
                key = (result['filepath'], sheet_name)
                entry = scored.setdefault(key, {'score': 0, 'terms': set(), 'filename': result['filename']})
                entry['score'] += 1
                entry['terms'].add(term)

    ranked = sorted(
        scored.items(),
        key=lambda item: (-item[1]['score'], item[1]['filename'].lower()),
    )
    return ranked[:max_results]


def _build_evidence(scanner: XlsxScanner, candidates, max_chars: int) -> str:
    """把候选子表和命中单元格整理成给模型的证据。"""
    from openpyxl.utils import get_column_letter

    blocks = []
    used_chars = 0
    for (filepath, sheet_name), data in candidates:
        if not os.path.exists(filepath):
            continue
        term = max(data['terms'], key=len) if data['terms'] else ''
        hits = []
        preview = []
        if term:
            try:
                hits, preview, _ = scanner.read_sheet_with_hits(
                    filepath, sheet_name,
                    keyword=term, match_mode='fuzzy',
                    max_hits=6, preview_rows=5, preview_cols=10,
                )
            except Exception:
                pass

        lines = [
            f"[文件] {os.path.basename(filepath)}",
            f"[子表] {sheet_name}",
        ]
        for hit in hits[:5]:
            coord = f"{get_column_letter(hit['col'])}{hit['row']}"
            lines.append(f"  [{coord}] {hit.get('value', '')}")
        for row in preview[:3]:
            cells = [cell for cell in row if cell != '']
            if cells:
                lines.append('  ' + ' | '.join(cells))

        block = '\n'.join(lines)
        if used_chars + len(block) > max_chars and blocks:
            break
        blocks.append(block)
        used_chars += len(block)
    return '\n\n'.join(blocks)


def _build_ask_messages(question: str, evidence: str) -> List[Dict]:
    system = (
        '你是本地 Excel 配置表的检索问答助手。'
        '只依据用户提供的“检索证据”回答，不要编造数据和文件名。'
        '回答时标注出处，格式为 [文件/子表/单元格]。'
        '如果证据不足以回答，明确说明证据不足，不要猜测。'
    )
    user = f'问题：{question}\n\n检索证据：\n{evidence}'
    return [
        {'role': 'system', 'content': system},
        {'role': 'user', 'content': user},
    ]


def _call_llm(messages: List[Dict], args) -> str:
    api_key = os.environ.get('XLSXSEARCHER_API_KEY') or os.environ.get('OPENAI_API_KEY')
    if not api_key:
        raise CliError('未配置 API Key，请设置 OPENAI_API_KEY 或 XLSXSEARCHER_API_KEY')

    base_url = (
        args.base_url
        or os.environ.get('XLSXSEARCHER_BASE_URL')
        or os.environ.get('OPENAI_BASE_URL')
        or 'https://api.openai.com/v1'
    ).rstrip('/')
    model = (
        args.model
        or os.environ.get('XLSXSEARCHER_MODEL')
        or os.environ.get('OPENAI_MODEL')
        or _DEFAULT_MODEL
    )

    payload = {'model': model, 'messages': messages, 'temperature': 0}
    request = urllib.request.Request(
        f'{base_url}/chat/completions',
        data=json.dumps(payload).encode('utf-8'),
        headers={
            'Content-Type': 'application/json',
            'Authorization': f'Bearer {api_key}',
        },
        method='POST',
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            body = json.loads(response.read().decode('utf-8'))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode('utf-8', errors='replace')
        raise CliError(f'AI 请求失败 {exc.code}: {detail[:500]}')
    except urllib.error.URLError as exc:
        raise CliError(f'AI 请求失败: {exc.reason}')

    try:
        return body['choices'][0]['message']['content'].strip()
    except (KeyError, IndexError, TypeError):
        raise CliError(f'AI 响应格式异常: {json.dumps(body, ensure_ascii=False)[:500]}')


def cmd_ask(args) -> int:
    manager = _make_manager(args)
    status = manager.get_index_status()
    if status['file_count'] == 0:
        raise CliError('索引为空，请先运行 scan')
    if status['pending_deep_index_count'] > 0:
        print(
            f'提示: 还有 {status["pending_deep_index_count"]} 个子表未深度索引，'
            '答案可能不完整（可运行 deep-index）',
            file=sys.stderr,
        )

    candidates = _retrieve_candidates(manager, args.question, args.max_results)
    if not candidates:
        raise CliError('未检索到与问题相关的子表内容，请换一种问法或先执行 scan/deep-index')

    scanner = XlsxScanner(use_calamine=True)
    evidence = _build_evidence(scanner, candidates, args.max_context_chars)
    messages = _build_ask_messages(args.question, evidence)

    if args.dry_run:
        if args.format == 'json':
            json.dump(
                {'question': args.question, 'evidence': evidence, 'messages': messages},
                sys.stdout,
                ensure_ascii=False,
                indent=2,
            )
            sys.stdout.write('\n')
        else:
            print(evidence)
            print('\n--- 模型请求（--dry-run，未发送）---')
            print(json.dumps(messages, ensure_ascii=False, indent=2))
        return 0

    answer = _call_llm(messages, args)
    if args.format == 'json':
        json.dump(
            {'question': args.question, 'evidence': evidence, 'answer': answer},
            sys.stdout,
            ensure_ascii=False,
            indent=2,
        )
        sys.stdout.write('\n')
    else:
        print(answer)
    return 0


def build_parser() -> argparse.ArgumentParser:
    app_version = _load_app_config().get('version', '0.0.0')
    parser = argparse.ArgumentParser(
        prog='xlsxsearcher',
        description='XlsxSearcher CLI - 本地 Excel 子表搜索与问答',
    )
    parser.add_argument(
        '-V', '--version',
        action='version',
        version=f'%(prog)s {app_version}',
    )
    db_parser = argparse.ArgumentParser(add_help=False)
    db_parser.add_argument('--db', default=None, help='SQLite 索引数据库路径')
    subparsers = parser.add_subparsers(dest='command', required=True)

    version_parser = subparsers.add_parser('version', help='显示版本号')
    version_parser.set_defaults(func=cmd_version)

    scan_parser = subparsers.add_parser('scan', parents=[db_parser], help='增量扫描目录')
    scan_parser.add_argument('directory', help='要扫描的目录')
    scan_parser.add_argument('--quiet', action='store_true', help='不显示进度')
    scan_parser.set_defaults(func=cmd_scan)

    deep_parser = subparsers.add_parser('deep-index', parents=[db_parser], help='提取未索引的单元格内容')
    deep_parser.add_argument('--processes', type=int, default=0, help='子进程数')
    deep_parser.add_argument('--quiet', action='store_true', help='不显示进度')
    deep_parser.set_defaults(func=cmd_deep_index)

    search_parser = subparsers.add_parser('search', parents=[db_parser], help='搜索文件与子表')
    _add_search_args(search_parser)
    search_parser.add_argument('--format', choices=['text', 'json', 'csv'], default='text')
    search_parser.add_argument('--export', default=None, help='导出 CSV 文件路径')
    search_parser.set_defaults(func=cmd_search)

    export_parser = subparsers.add_parser('export', parents=[db_parser], help='导出搜索结果为 CSV')
    _add_search_args(export_parser)
    export_parser.add_argument('output', help='输出 CSV 文件路径')
    export_parser.set_defaults(func=cmd_export)

    alias_parser = subparsers.add_parser('alias', parents=[db_parser], help='管理别名映射')
    alias_subparsers = alias_parser.add_subparsers(dest='alias_command', required=True)
    import_parser = alias_subparsers.add_parser('import', help='导入映射文件')
    import_parser.add_argument('file', help='bat/txt 映射文件路径')
    import_parser.set_defaults(alias_command='import')
    list_parser = alias_subparsers.add_parser('list', help='列出全部别名映射')
    list_parser.set_defaults(alias_command='list')
    alias_parser.set_defaults(func=cmd_alias)

    stats_parser = subparsers.add_parser('stats', parents=[db_parser], help='查看索引统计')
    stats_parser.add_argument('--format', choices=['text', 'json'], default='text')
    stats_parser.set_defaults(func=cmd_stats)

    ask_parser = subparsers.add_parser('ask', parents=[db_parser], help='基于索引的自然语言问答')
    ask_parser.add_argument('question', help='要问的问题，例如 "ItemConfig 配置了哪些道具"')
    ask_parser.add_argument('--format', choices=['text', 'json'], default='text')
    ask_parser.add_argument('--max-results', type=int, default=8, help='最多召回多少个子表')
    ask_parser.add_argument('--max-context-chars', type=int, default=12000, help='发送给模型的证据上限字符数')
    ask_parser.add_argument('--model', default=None, help='模型名，默认读 XLSXSEARCHER_MODEL/OPENAI_MODEL')
    ask_parser.add_argument('--base-url', default=None, help='OpenAI 兼容接口地址')
    ask_parser.add_argument('--dry-run', action='store_true', help='只输出检索证据和请求内容，不调用模型')
    ask_parser.set_defaults(func=cmd_ask)

    return parser


def main(argv=None) -> int:
    if hasattr(sys.stdout, 'reconfigure'):
        sys.stdout.reconfigure(encoding='utf-8', errors='replace')
        sys.stderr.reconfigure(encoding='utf-8', errors='replace')
    multiprocessing.freeze_support()

    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except KeyboardInterrupt:
        print('\n已取消', file=sys.stderr)
        return 130
    except CliError as exc:
        print(f'错误: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
