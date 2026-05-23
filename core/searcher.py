"""搜索器 - 提供搜索接口"""
from typing import List, Dict
from core.indexer import IndexManager

class Searcher:
    def __init__(self, index_manager: IndexManager):
        self.index_manager = index_manager

    def _resolve_sheet_keywords(self, keyword: str, match_mode: str) -> List[str]:
        """展开用户输入的子表关键字，兼容导入的英文别名。"""
        if not keyword:
            return []

        keywords = [keyword]
        keywords.extend(self.index_manager.resolve_sheet_aliases(keyword, match_mode))

        resolved = []
        seen = set()
        for item in keywords:
            normalized = (item or '').strip()
            if not normalized:
                continue
            dedupe_key = normalized.lower()
            if dedupe_key in seen:
                continue
            seen.add(dedupe_key)
            resolved.append(normalized)
        return resolved

    def search(
        self,
        sheet_keyword: str = None,
        filename_keyword: str = None,
        cell_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        """
        搜索xlsx文件
        @param sheet_keyword: 子表名称关键字
        @param filename_keyword: 文件名关键字
        @param cell_keyword: 单元格内容关键字
        @param match_mode: 匹配模式 exact / prefix / fuzzy
        @return: 搜索结果列表
        """
        if not sheet_keyword and not filename_keyword and not cell_keyword:
            return []

        return self.index_manager.search(
            self._resolve_sheet_keywords(sheet_keyword, match_mode),
            filename_keyword,
            cell_keyword,
            match_mode
        )

    def search_by_sheet_name(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """仅按子表名称搜索"""
        return self.index_manager.search(self._resolve_sheet_keywords(keyword, match_mode), match_mode=match_mode)

    def search_by_filename(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """仅按文件名搜索"""
        return self.index_manager.search_by_filename(keyword, match_mode)
