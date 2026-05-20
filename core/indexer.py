"""索引管理器 - 使用SQLite存储xlsx文件索引"""
import sqlite3
import os
from typing import List, Dict, Tuple

class IndexManager:
    def __init__(self, db_path: str = None):
        if db_path is None:
            # 在用户目录创建数据库
            user_home = os.path.expanduser("~")
            app_data_dir = os.path.join(user_home, ".local", "XlsxSearcher")
            os.makedirs(app_data_dir, exist_ok=True)
            db_path = os.path.join(app_data_dir, "index.db")
        self.db_path = db_path
        self._init_db()

    def _init_db(self):
        """初始化数据库表"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        # 创建xlsx文件索引表
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS xlsx_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                filename TEXT NOT NULL,
                filepath TEXT UNIQUE NOT NULL,
                modified_time REAL NOT NULL,
                sheet_count INTEGER DEFAULT 0
            )
        ''')
        # 创建子表索引表
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS sheets (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                sheet_name TEXT NOT NULL,
                FOREIGN KEY (file_id) REFERENCES xlsx_files(id) ON DELETE CASCADE
            )
        ''')
        # 创建索引
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_filename ON xlsx_files(filename)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_filepath ON xlsx_files(filepath)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_sheet_name ON sheets(sheet_name)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_file_id ON sheets(file_id)')

        # Migration: add cell_text column for cell content search
        cursor.execute("PRAGMA table_info(sheets)")
        columns = [col[1] for col in cursor.fetchall()]
        if 'cell_text' not in columns:
            cursor.execute('ALTER TABLE sheets ADD COLUMN cell_text TEXT')

        conn.commit()
        conn.close()

    def get_file_info(self, filepath: str) -> Tuple:
        """获取文件信息"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('SELECT id, modified_time FROM xlsx_files WHERE filepath = ?', (filepath,))
        result = cursor.fetchone()
        conn.close()
        return result  # (id, modified_time) or None

    def add_file(self, filename: str, filepath: str, modified_time: float,
                 sheet_names: List[str], cell_texts: List[str] = None):
        """添加文件及其子表到索引，可选附带单元格内容"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()

        # 先查询是否已存在，获取 file_id
        cursor.execute('SELECT id FROM xlsx_files WHERE filepath = ?', (filepath,))
        row = cursor.fetchone()

        if row:
            # 已存在，更新并获取 file_id
            file_id = row[0]
            cursor.execute(
                'UPDATE xlsx_files SET filename = ?, modified_time = ?, sheet_count = ? WHERE id = ?',
                (filename, modified_time, len(sheet_names), file_id)
            )
            # 删除旧的子表
            cursor.execute('DELETE FROM sheets WHERE file_id = ?', (file_id,))
        else:
            # 新插入
            cursor.execute(
                'INSERT INTO xlsx_files (filename, filepath, modified_time, sheet_count) VALUES (?, ?, ?, ?)',
                (filename, filepath, modified_time, len(sheet_names))
            )
            file_id = cursor.lastrowid

        # 插入子表信息
        for i, sheet_name in enumerate(sheet_names):
            cell_text = cell_texts[i] if cell_texts and i < len(cell_texts) else None
            cursor.execute(
                'INSERT INTO sheets (file_id, sheet_name, cell_text) VALUES (?, ?, ?)',
                (file_id, sheet_name, cell_text)
            )
        conn.commit()
        conn.close()

    def delete_file(self, filepath: str):
        """从索引中删除文件"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('SELECT id FROM xlsx_files WHERE filepath = ?', (filepath,))
        row = cursor.fetchone()
        if row:
            file_id = row[0]
            cursor.execute('DELETE FROM sheets WHERE file_id = ?', (file_id,))
            cursor.execute('DELETE FROM xlsx_files WHERE id = ?', (file_id,))
        conn.commit()
        conn.close()

    def get_all_files(self) -> List[Dict]:
        """获取所有已索引的文件"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('SELECT id, filename, filepath, modified_time, sheet_count FROM xlsx_files')
        rows = cursor.fetchall()
        conn.close()
        return [
            {'id': r[0], 'filename': r[1], 'filepath': r[2], 'modified_time': r[3], 'sheet_count': r[4]}
            for r in rows
        ]

    def _build_match_clause(self, field_name: str, keyword: str, match_mode: str) -> Tuple[str, str]:
        normalized_mode = match_mode or 'fuzzy'
        if normalized_mode == 'exact':
            return f"LOWER({field_name}) = LOWER(?)", keyword
        if normalized_mode == 'prefix':
            return f"{field_name} LIKE ? COLLATE NOCASE", f'{keyword}%'
        return f"{field_name} LIKE ? COLLATE NOCASE", f'%{keyword}%'

    def _fetch_grouped_results(
        self,
        sheet_keyword: str = None,
        filename_keyword: str = None,
        cell_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()

        query = [
            'SELECT f.filename, f.filepath, s.sheet_name',
            'FROM xlsx_files f',
            'LEFT JOIN sheets s ON f.id = s.file_id'
        ]
        conditions = []
        params = []

        if filename_keyword:
            clause, value = self._build_match_clause('f.filename', filename_keyword, match_mode)
            conditions.append(clause)
            params.append(value)

        if sheet_keyword:
            clause, value = self._build_match_clause('s.sheet_name', sheet_keyword, match_mode)
            conditions.append(clause)
            params.append(value)

        if cell_keyword:
            clause, value = self._build_match_clause('s.cell_text', cell_keyword, match_mode)
            conditions.append(clause)
            params.append(value)

        if conditions:
            query.append('WHERE ' + ' AND '.join(conditions))

        query.append('ORDER BY LOWER(f.filename), LOWER(s.sheet_name)')
        cursor.execute('\n'.join(query), tuple(params))
        rows = cursor.fetchall()
        conn.close()

        grouped = {}
        for filename, filepath, sheet_name in rows:
            entry = grouped.setdefault(
                filepath,
                {
                    'filename': filename,
                    'filepath': filepath,
                    'sheet_names': []
                }
            )
            if sheet_name:
                entry['sheet_names'].append(sheet_name)

        results = list(grouped.values())
        for result in results:
            result['sheet_count'] = len(result['sheet_names'])
            result['sheet_names_display'] = ', '.join(result['sheet_names'])
        return results

    def get_all_files_with_sheets(self) -> List[Dict]:
        """获取所有已索引文件及其子表"""
        return self._fetch_grouped_results()

    def search_by_sheet_name(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """按子表名称搜索"""
        return self._fetch_grouped_results(sheet_keyword=keyword, match_mode=match_mode)

    def search_by_filename(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """按文件名搜索"""
        return self._fetch_grouped_results(filename_keyword=keyword, match_mode=match_mode)

    def search(
        self,
        sheet_keyword: str = None,
        filename_keyword: str = None,
        cell_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        """综合搜索"""
        if not sheet_keyword and not filename_keyword and not cell_keyword:
            return []
        return self._fetch_grouped_results(
            sheet_keyword=sheet_keyword,
            filename_keyword=filename_keyword,
            cell_keyword=cell_keyword,
            match_mode=match_mode
        )

    def get_sheets_without_cell_text(self) -> List[Dict]:
        """获取 cell_text 为 NULL 的 sheet 列表，用于深度索引"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('''
            SELECT f.filepath, s.sheet_name, s.id
            FROM sheets s
            JOIN xlsx_files f ON s.file_id = f.id
            WHERE s.cell_text IS NULL
        ''')
        rows = cursor.fetchall()
        conn.close()
        return [{'filepath': r[0], 'sheet_name': r[1], 'sheet_id': r[2]} for r in rows]

    def update_sheet_cell_text(self, sheet_id: int, cell_text: str):
        """更新单个 sheet 的 cell_text"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('UPDATE sheets SET cell_text = ? WHERE id = ?', (cell_text, sheet_id))
        conn.commit()
        conn.close()

    def update_sheet_cell_texts_batch(self, updates: List[Tuple[int, str]]):
        """批量更新 sheet 的 cell_text（单事务，一次连接）"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('BEGIN')
        for sheet_id, cell_text in updates:
            cursor.execute('UPDATE sheets SET cell_text = ? WHERE id = ?', (cell_text, sheet_id))
        conn.commit()
        conn.close()

    def clear_index(self):
        """清空所有索引"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('DELETE FROM sheets')
        cursor.execute('DELETE FROM xlsx_files')
        conn.commit()
        conn.close()

    def get_stats(self) -> Dict:
        """获取索引统计信息"""
        conn = sqlite3.connect(self.db_path)
        cursor = conn.cursor()
        cursor.execute('SELECT COUNT(*) FROM xlsx_files')
        file_count = cursor.fetchone()[0]
        cursor.execute('SELECT COUNT(*) FROM sheets')
        sheet_count = cursor.fetchone()[0]
        conn.close()
        return {'file_count': file_count, 'sheet_count': sheet_count}
