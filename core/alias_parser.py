"""映射脚本解析器 - 从配置文件中提取英文名与子表名映射

支持两种格式：

标准格式（推荐）:
    # 以 # 开头的行为注释
    EnglishConfigName SheetName1 SheetName2 ...
    示例: TextConfig 界面文本 UI Text

旧格式（兼容）:
    call <script> <AliasNames...> <SheetNames...>
    示例: call do_conv.bat TextConfig 界面文本
"""
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


def _is_comment(line: str) -> bool:
    """判断是否为注释行。"""
    return line.startswith('#') or line.startswith('::') or line.upper().startswith('REM ')


def _parse_new_format(tokens: List[str]) -> List[Tuple[str, str]]:
    """解析标准格式: EnglishName SheetName1 SheetName2 ...

    首列为英文配置名，其余列为子表名。返回 (alias_name, sheet_name) 列表。
    """
    alias_name = tokens[0]
    if not ASCII_TOKEN_RE.match(alias_name):
        return []
    sheet_names = tokens[1:]
    if not sheet_names:
        return []
    return [(alias_name, sheet_name) for sheet_name in sheet_names]


def _parse_legacy_format(tokens: List[str]) -> List[Tuple[str, str]]:
    """解析旧格式: call <script> <ASCII_alias...> <sheet_names...>

    在 payload 中，第一个非 ASCII token 之前的是别名，之后的是子表名。
    所有别名与所有子表名做笛卡尔积。
    """
    payload_tokens = tokens[2:]
    alias_names = []
    sheet_names = []

    for token in payload_tokens:
        if not sheet_names and ASCII_TOKEN_RE.match(token):
            alias_names.append(token)
            continue
        sheet_names.append(token)

    if not alias_names or not sheet_names:
        return []

    mappings = []
    for alias_name in alias_names:
        for sheet_name in sheet_names:
            mappings.append((alias_name, sheet_name))
    return mappings


def parse_sheet_alias_file(file_path: str) -> List[Tuple[str, str]]:
    """解析映射文件，返回 (alias_name, sheet_name) 列表。

    支持格式：
    - 标准格式: EnglishConfigName SheetName1 SheetName2 ...
    - 旧格式:    call <script> AliasName... SheetName...

    以 #、::、REM 开头的行为注释。
    """
    mappings = []

    content = _read_text_with_fallbacks(file_path)
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line or _is_comment(line):
            continue

        tokens = line.split()
        if len(tokens) < 2:
            continue

        if tokens[0].lower() == 'call' and len(tokens) >= 3:
            line_mappings = _parse_legacy_format(tokens)
        else:
            line_mappings = _parse_new_format(tokens)

        mappings.extend(line_mappings)

    # 去重
    deduplicated = []
    seen = set()
    for alias_name, sheet_name in mappings:
        key = (alias_name.lower(), sheet_name)
        if key in seen:
            continue
        seen.add(key)
        deduplicated.append((alias_name, sheet_name))
    return deduplicated
