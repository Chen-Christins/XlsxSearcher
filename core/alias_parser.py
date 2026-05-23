"""映射脚本解析器 - 从 bat/txt 配置中提取英文名与子表名映射"""
import re
from typing import List, Tuple


ASCII_TOKEN_RE = re.compile(r'^[A-Za-z0-9_]+$')


def _read_text_with_fallbacks(file_path: str) -> str:
    """优先按常见 Windows 文本编码读取。"""
    encodings = ('utf-8-sig', 'utf-8', 'gbk', 'gb18030')
    last_error = None
    for encoding in encodings:
        try:
            with open(file_path, 'r', encoding=encoding) as file:
                return file.read()
        except UnicodeDecodeError as error:
            last_error = error
    if last_error:
        raise last_error
    raise UnicodeDecodeError('utf-8', b'', 0, 1, 'unable to decode file')


def parse_sheet_alias_file(file_path: str) -> List[Tuple[str, str]]:
    """解析映射脚本，返回 (alias_name, sheet_name) 列表。"""
    mappings = []

    content = _read_text_with_fallbacks(file_path)
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith('::') or line.upper().startswith('REM '):
            continue

        tokens = line.split()
        if len(tokens) < 3 or tokens[0].lower() != 'call':
            continue

        payload_tokens = tokens[2:]
        alias_names = []
        sheet_names = []

        for token in payload_tokens:
            if not sheet_names and ASCII_TOKEN_RE.match(token):
                alias_names.append(token)
                continue
            sheet_names.append(token)

        if not alias_names or not sheet_names:
            continue

        for alias_name in alias_names:
            for sheet_name in sheet_names:
                mappings.append((alias_name, sheet_name))

    deduplicated = []
    seen = set()
    for alias_name, sheet_name in mappings:
        key = (alias_name.lower(), sheet_name)
        if key in seen:
            continue
        seen.add(key)
        deduplicated.append((alias_name, sheet_name))
    return deduplicated
