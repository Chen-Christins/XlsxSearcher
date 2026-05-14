"""搜索器 - 提供搜索接口"""
from typing import List, Dict
from core.indexer import IndexManager

class Searcher:
    def __init__(self, index_manager: IndexManager):
        self.index_manager = index_manager

    def search(
        self,
        sheet_keyword: str = None,
        filename_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        """
        搜索xlsx文件
        @param sheet_keyword: 子表名称关键字
        @param filename_keyword: 文件名关键字
        @param match_mode: 匹配模式 exact / prefix / fuzzy
        @return: 搜索结果列表
        """
        if not sheet_keyword and not filename_keyword:
            return []

        return self.index_manager.search(sheet_keyword, filename_keyword, match_mode)

    def search_by_sheet_name(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """仅按子表名称搜索"""
        return self.index_manager.search_by_sheet_name(keyword, match_mode)

    def search_by_filename(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """仅按文件名搜索"""
        return self.index_manager.search_by_filename(keyword, match_mode)
