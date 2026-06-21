"""索引管理器 - 使用SQLite存储xlsx文件索引"""
import os
import sqlite3
import threading
from typing import Dict, List, Tuple

# FTS5 trigram 至少需要 3 个字符才能高效命中；更短的关键字回退到 LIKE。
_FTS_MIN_TOKEN_LEN = 3


class IndexManager:
    def __init__(self, db_path: str = None):
        if db_path is None:
            # 在用户目录创建数据库
            user_home = os.path.expanduser("~")
            app_data_dir = os.path.join(user_home, ".local", "XlsxSearcher")
            os.makedirs(app_data_dir, exist_ok=True)
            db_path = os.path.join(app_data_dir, "index.db")
        self.db_path = db_path
        # 每个线程持有一个长连接（WAL 允许多读 + 单写并发），避免每次操作都
        # connect/close 的开销。ThreadPoolExecutor 复用线程，连接数有界。
        self._tls = threading.local()
        self._init_db()

    # ---- 连接管理 ----

    def _conn(self) -> sqlite3.Connection:
        """返回当前线程的长连接，懒加载。"""
        conn = getattr(self._tls, 'conn', None)
        if conn is None:
            conn = sqlite3.connect(self.db_path)
            conn.execute('PRAGMA journal_mode=WAL')
            conn.execute('PRAGMA synchronous=NORMAL')
            conn.execute('PRAGMA foreign_keys=ON')
            self._tls.conn = conn
        return conn

    def close_thread_connection(self):
        """显式关闭当前线程的连接（可选，应用退出时调用）。"""
        conn = getattr(self._tls, 'conn', None)
        if conn is not None:
            conn.close()
            del self._tls.conn

    def _init_db(self):
        """初始化数据库表"""
        conn = self._conn()
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
        cursor.execute('''
            CREATE TABLE IF NOT EXISTS sheet_aliases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                alias_name TEXT NOT NULL,
                sheet_name TEXT NOT NULL,
                source_path TEXT NOT NULL,
                UNIQUE(alias_name, sheet_name, source_path)
            )
        ''')
        # 创建索引
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_filename ON xlsx_files(filename)')
        # idx_filepath 冗余：filepath 已有 UNIQUE 约束自带索引
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_sheet_name ON sheets(sheet_name)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_file_id ON sheets(file_id)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_alias_name ON sheet_aliases(alias_name)')
        cursor.execute('CREATE INDEX IF NOT EXISTS idx_alias_sheet_name ON sheet_aliases(sheet_name)')

        # Migration: add cell_text column for cell content search
        cursor.execute("PRAGMA table_info(sheets)")
        columns = [col[1] for col in cursor.fetchall()]
        if 'cell_text' not in columns:
            cursor.execute('ALTER TABLE sheets ADD COLUMN cell_text TEXT')

        self._init_fts(cursor)

        conn.commit()

    def _init_fts(self, cursor):
        """创建 FTS5 trigram 全文索引（外部内容表，挂在 sheets.cell_text 上）。

        trigram 分词器支持子串匹配，等价于 LIKE '%kw%'，但能走倒排索引，
        在 cell_text 较大 / sheet 较多时比全表 LIKE 快数个数量级。
        通过触发器与 sheets 表保持同步，upsert / deep-index 无需额外改动。
        """
        cursor.execute('''
            CREATE VIRTUAL TABLE IF NOT EXISTS sheets_fts USING fts5(
                cell_text, content='sheets', content_rowid='id', tokenize='trigram'
            )
        ''')
        cursor.execute('''
            CREATE TRIGGER IF NOT EXISTS sheets_fts_ai AFTER INSERT ON sheets BEGIN
                INSERT INTO sheets_fts(rowid, cell_text) VALUES (new.id, new.cell_text);
            END
        ''')
        cursor.execute('''
            CREATE TRIGGER IF NOT EXISTS sheets_fts_ad AFTER DELETE ON sheets BEGIN
                INSERT INTO sheets_fts(sheets_fts, rowid, cell_text)
                VALUES('delete', old.id, old.cell_text);
            END
        ''')
        cursor.execute('''
            CREATE TRIGGER IF NOT EXISTS sheets_fts_au AFTER UPDATE ON sheets BEGIN
                INSERT INTO sheets_fts(sheets_fts, rowid, cell_text)
                VALUES('delete', old.id, old.cell_text);
                INSERT INTO sheets_fts(rowid, cell_text) VALUES (new.id, new.cell_text);
            END
        ''')
        # 回填：仅对尚未进入 FTS 的行建索引（幂等，可重复执行）。
        cursor.execute('''
            INSERT INTO sheets_fts(rowid, cell_text)
            SELECT id, cell_text FROM sheets
            WHERE id NOT IN (SELECT rowid FROM sheets_fts)
        ''')

    def get_file_info(self, filepath: str) -> Tuple:
        """获取文件信息"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT id, modified_time FROM xlsx_files WHERE filepath = ?', (filepath,))
        return cursor.fetchone()  # (id, modified_time) or None

    def get_all_files_indexed(self) -> Dict[str, Tuple[int, float]]:
        """一次查询拿到 filepath -> (id, modified_time) 的快照。

        比 N 次 get_file_info 少 N-1 次连接 + 查询。
        """
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT id, filepath, modified_time FROM xlsx_files')
        rows = cursor.fetchall()
        return {r[1]: (r[0], r[2]) for r in rows}

    def add_file(self, filename: str, filepath: str, modified_time: float,
                 sheet_names: List[str], cell_texts: List[str] = None):
        """添加文件及其子表到索引，可选附带单元格内容"""
        conn = self._conn()
        cursor = conn.cursor()

        # 先查询是否已存在，获取 file_id
        cursor.execute('SELECT id FROM xlsx_files WHERE filepath = ?', (filepath,))
        row = cursor.fetchone()

        # 查询已有 cell_text 以便保留
        old_cell_texts = {}
        if row:
            file_id = row[0]
            cursor.execute(
                'SELECT sheet_name, cell_text FROM sheets WHERE file_id = ?', (file_id,)
            )
            old_cell_texts = {r[0]: r[1] for r in cursor.fetchall()}

        if row:
            # 已存在，更新并获取 file_id
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

        # 插入子表信息（批量），优先使用传入的 cell_texts，其次保留旧的
        if cell_texts:
            extra = [None] * (len(sheet_names) - len(cell_texts))
            cell_texts = cell_texts + extra
        else:
            cell_texts = [
                old_cell_texts.get(sheet_name)
                for sheet_name in sheet_names
            ]
        cursor.executemany(
            'INSERT INTO sheets (file_id, sheet_name, cell_text) VALUES (?, ?, ?)',
            [(file_id, s, c) for s, c in zip(sheet_names, cell_texts)]
        )
        conn.commit()

    def upsert_files_batch(self, updates: List[Tuple[str, str, float, List[str]]],
                            indexed: Dict[str, Tuple[int, float]] = None
                            ) -> Tuple[int, int]:
        """在单个事务里批量 upsert 文件及其子表。

        updates: [(filename, filepath, mtime, sheet_names), ...]
        indexed: 当前索引快照 filepath -> (id, mtime)；提供时无需回查即可区分 add/update。
        返回 (added, updated) 计数。

        优化：对于 sheet 名列表与现有完全一致的更新文件，跳过 sheets 的
        DELETE+INSERT 重建，只更新 xlsx_files 的 mtime —— 避免无谓的写放大，
        也免去搬运已深度索引的 cell_text。
        """
        if not updates:
            return 0, 0

        conn = self._conn()
        cursor = conn.cursor()

        # 一次性回查 index 中已有的 file_id（如果调用方没传快照）
        if indexed is None:
            filepaths = [u[1] for u in updates]
            placeholders = ','.join('?' * len(filepaths))
            cursor.execute(
                f'SELECT filepath, id FROM xlsx_files WHERE filepath IN ({placeholders})',
                filepaths
            )
            indexed = {fp: (row_id, None) for fp, row_id in cursor.fetchall()}

        added = 0
        updated = 0
        sheet_inserts = []
        file_ids_to_clear = []

        # 对于要更新的文件，先查询现有 cell_text 和 sheet 名顺序，以便保留与比对
        old_cell_texts = {}  # (file_id, sheet_name) -> cell_text
        existing_sheet_names = {}  # file_id -> [sheet_name, ...] (按 id 排序)
        update_file_ids = []
        for filename, filepath, mtime, sheet_names in updates:
            existing = indexed.get(filepath)
            if existing is not None:
                update_file_ids.append(existing[0])

        if update_file_ids:
            placeholders = ','.join('?' * len(update_file_ids))
            cursor.execute(
                f'SELECT file_id, sheet_name, cell_text FROM sheets WHERE file_id IN ({placeholders})',
                update_file_ids
            )
            tmp_names = {}
            for row in cursor.fetchall():
                fid, sname, ctext = row
                old_cell_texts[(fid, sname)] = ctext
                tmp_names.setdefault(fid, []).append(sname)
            # 按 id 排序重建顺序（上面查询未保证顺序）
            cursor.execute(
                f'SELECT file_id, sheet_name FROM sheets WHERE file_id IN ({placeholders}) ORDER BY file_id, id',
                update_file_ids
            )
            for row in cursor.fetchall():
                existing_sheet_names.setdefault(row[0], []).append(row[1])

        for filename, filepath, mtime, sheet_names in updates:
            existing = indexed.get(filepath)
            if existing is not None:
                file_id = existing[0]
                cursor.execute(
                    'UPDATE xlsx_files SET filename=?, modified_time=?, sheet_count=? WHERE id=?',
                    (filename, mtime, len(sheet_names), file_id)
                )
                # sheet 名列表完全一致 → 跳过重建，保留现有 cell_text
                if existing_sheet_names.get(file_id) == list(sheet_names):
                    updated += 1
                    continue
                file_ids_to_clear.append(file_id)
                updated += 1
            else:
                cursor.execute(
                    'INSERT INTO xlsx_files (filename, filepath, modified_time, sheet_count) VALUES (?, ?, ?, ?)',
                    (filename, filepath, mtime, len(sheet_names))
                )
                file_id = cursor.lastrowid
                added += 1
            for sheet_name in sheet_names:
                preserved = old_cell_texts.get((file_id, sheet_name))
                sheet_inserts.append((file_id, sheet_name, preserved))

        if file_ids_to_clear:
            placeholders = ','.join('?' * len(file_ids_to_clear))
            cursor.execute(
                f'DELETE FROM sheets WHERE file_id IN ({placeholders})',
                file_ids_to_clear
            )

        if sheet_inserts:
            cursor.executemany(
                'INSERT INTO sheets (file_id, sheet_name, cell_text) VALUES (?, ?, ?)',
                sheet_inserts
            )

        conn.commit()
        return added, updated

    def delete_file(self, filepath: str):
        """从索引中删除文件"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT id FROM xlsx_files WHERE filepath = ?', (filepath,))
        row = cursor.fetchone()
        if row:
            file_id = row[0]
            cursor.execute('DELETE FROM sheets WHERE file_id = ?', (file_id,))
            cursor.execute('DELETE FROM xlsx_files WHERE id = ?', (file_id,))
        conn.commit()

    def delete_files_by_ids(self, file_ids: List[int]) -> int:
        """按 id 批量删除文件（及其子表），单事务。返回删除条数。"""
        if not file_ids:
            return 0
        conn = self._conn()
        cursor = conn.cursor()
        placeholders = ','.join('?' * len(file_ids))
        cursor.execute(f'DELETE FROM sheets WHERE file_id IN ({placeholders})', file_ids)
        cursor.execute(f'DELETE FROM xlsx_files WHERE id IN ({placeholders})', file_ids)
        conn.commit()
        return len(file_ids)

    def get_all_files(self) -> List[Dict]:
        """获取所有已索引的文件"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT id, filename, filepath, modified_time, sheet_count FROM xlsx_files')
        rows = cursor.fetchall()
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

    @staticmethod
    def _fts_match_value(keyword: str) -> str:
        """把关键字转成 FTS5 trigram 短语查询：用双引号包裹，内部双引号转义。"""
        return '"' + keyword.replace('"', '""') + '"'

    def _build_cell_condition(self, keyword: str, match_mode: str) -> Tuple[str, List[str]]:
        """构造 cell_text 命中条件。

        - 关键字 >= 3 字符：走 FTS5 trigram（子串匹配，等价 LIKE '%kw%' 但走倒排索引）。
          prefix/exact 在 FTS 缩小候选集后再用原 LIKE 收紧，保持语义不变。
        - 关键字 < 3 字符：trigram 无法高效命中，回退到全表 LIKE（行为与旧版一致）。
        返回 (SQL 片段, params)。
        """
        normalized_mode = match_mode or 'fuzzy'
        if len(keyword) >= _FTS_MIN_TOKEN_LEN:
            fts_subquery = (
                'SELECT rowid FROM sheets_fts WHERE sheets_fts MATCH ?'
            )
            cond = f's.id IN ({fts_subquery})'
            params = [self._fts_match_value(keyword)]
            if normalized_mode == 'prefix':
                cond += ' AND s.cell_text LIKE ? COLLATE NOCASE'
                params.append(f'{keyword}%')
            elif normalized_mode == 'exact':
                cond += ' AND LOWER(s.cell_text) = LOWER(?)'
                params.append(keyword)
            # fuzzy: FTS 子串已等价于 LIKE '%kw%'，无需再加 LIKE
            return cond, params
        # 回退：短关键字走 LIKE
        clause, value = self._build_match_clause('s.cell_text', keyword, normalized_mode)
        return clause, [value]

    def replace_sheet_aliases(self, source_path: str, mappings: List[Tuple[str, str]]) -> int:
        """替换同一来源文件导入的子表别名映射。"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('DELETE FROM sheet_aliases WHERE source_path = ?', (source_path,))
        inserted = 0
        if mappings:
            cursor.executemany(
                'INSERT OR IGNORE INTO sheet_aliases (alias_name, sheet_name, source_path) VALUES (?, ?, ?)',
                [(alias_name, sheet_name, source_path) for alias_name, sheet_name in mappings]
            )
            cursor.execute('SELECT COUNT(*) FROM sheet_aliases WHERE source_path = ?', (source_path,))
            inserted = cursor.fetchone()[0]
        conn.commit()
        return inserted

    def resolve_sheet_aliases(self, keyword: str, match_mode: str = 'fuzzy') -> List[str]:
        """根据英文别名查询对应的子表名。"""
        if not keyword:
            return []

        conn = self._conn()
        cursor = conn.cursor()
        clause, value = self._build_match_clause('alias_name', keyword, match_mode)
        cursor.execute(
            f'SELECT DISTINCT sheet_name FROM sheet_aliases WHERE {clause} ORDER BY LOWER(sheet_name)',
            (value,)
        )
        rows = cursor.fetchall()
        return [row[0] for row in rows]

    def get_sheet_alias_stats(self) -> Dict:
        """获取已导入别名统计。"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT COUNT(DISTINCT alias_name), COUNT(*) FROM sheet_aliases')
        row = cursor.fetchone()
        return {
            'alias_count': row[0] if row else 0,
            'mapping_count': row[1] if row else 0,
        }

    def get_index_status(self) -> Dict:
        """返回索引覆盖情况，供状态栏和提示文案使用。"""
        conn = self._conn()
        cursor = conn.cursor()

        cursor.execute('SELECT COUNT(*) FROM xlsx_files')
        file_count = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM sheets')
        sheet_count = cursor.fetchone()[0]

        cursor.execute('SELECT COUNT(*) FROM sheets WHERE cell_text IS NULL')
        pending_deep_index_count = cursor.fetchone()[0]

        return {
            'file_count': file_count,
            'sheet_count': sheet_count,
            'pending_deep_index_count': pending_deep_index_count,
            'indexed_cell_sheet_count': max(sheet_count - pending_deep_index_count, 0),
        }

    def _fetch_grouped_results(
        self,
        sheet_keywords: List[str] = None,
        filename_keyword: str = None,
        cell_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        conn = self._conn()
        cursor = conn.cursor()

        query = [
            'SELECT DISTINCT f.filename, f.filepath, s.sheet_name',
            'FROM xlsx_files f',
            'LEFT JOIN sheets s ON f.id = s.file_id'
        ]
        conditions = []
        params = []

        if filename_keyword:
            clause, value = self._build_match_clause('f.filename', filename_keyword, match_mode)
            conditions.append(clause)
            params.append(value)

        if sheet_keywords:
            sheet_conditions = []
            seen_keywords = set()
            for sheet_keyword in sheet_keywords:
                normalized_keyword = (sheet_keyword or '').strip()
                if not normalized_keyword:
                    continue
                dedupe_key = normalized_keyword.lower()
                if dedupe_key in seen_keywords:
                    continue
                seen_keywords.add(dedupe_key)
                clause, value = self._build_match_clause('s.sheet_name', normalized_keyword, match_mode)
                sheet_conditions.append(clause)
                params.append(value)
            if sheet_conditions:
                conditions.append('(' + ' OR '.join(sheet_conditions) + ')')

        if cell_keyword:
            cell_cond, cell_params = self._build_cell_condition(cell_keyword, match_mode)
            conditions.append(cell_cond)
            params.extend(cell_params)

        if conditions:
            query.append('WHERE ' + ' AND '.join(conditions))

        query.append('ORDER BY LOWER(f.filename), LOWER(s.sheet_name)')
        cursor.execute('\n'.join(query), tuple(params))
        rows = cursor.fetchall()

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
        return self._fetch_grouped_results(sheet_keywords=[keyword], match_mode=match_mode)

    def search_by_filename(self, keyword: str, match_mode: str = 'fuzzy') -> List[Dict]:
        """按文件名搜索"""
        return self._fetch_grouped_results(filename_keyword=keyword, match_mode=match_mode)

    def search(
        self,
        sheet_keywords: List[str] = None,
        filename_keyword: str = None,
        cell_keyword: str = None,
        match_mode: str = 'fuzzy'
    ) -> List[Dict]:
        """综合搜索"""
        if not sheet_keywords and not filename_keyword and not cell_keyword:
            return []
        return self._fetch_grouped_results(
            sheet_keywords=sheet_keywords,
            filename_keyword=filename_keyword,
            cell_keyword=cell_keyword,
            match_mode=match_mode
        )

    def get_sheets_without_cell_text(self) -> List[Dict]:
        """获取 cell_text 为 NULL 的 sheet 列表，用于深度索引"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('''
            SELECT f.filepath, s.sheet_name, s.id
            FROM sheets s
            JOIN xlsx_files f ON s.file_id = f.id
            WHERE s.cell_text IS NULL
        ''')
        rows = cursor.fetchall()
        return [{'filepath': r[0], 'sheet_name': r[1], 'sheet_id': r[2]} for r in rows]

    def update_sheet_cell_text(self, sheet_id: int, cell_text: str):
        """更新单个 sheet 的 cell_text（FTS 由触发器自动同步）"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('UPDATE sheets SET cell_text = ? WHERE id = ?', (cell_text, sheet_id))
        conn.commit()

    def update_sheet_cell_texts_batch(self, updates: List[Tuple[str, int]]):
        """批量更新 sheet 的 cell_text（单事务，FTS 由触发器自动同步）

        updates: [(cell_text, sheet_id), ...]
        """
        conn = self._conn()
        cursor = conn.cursor()
        cursor.executemany('UPDATE sheets SET cell_text = ? WHERE id = ?', updates)
        conn.commit()

    def clear_index(self):
        """清空所有索引"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('DELETE FROM sheets')
        cursor.execute('DELETE FROM xlsx_files')
        conn.commit()

    def get_stats(self) -> Dict:
        """获取索引统计信息"""
        conn = self._conn()
        cursor = conn.cursor()
        cursor.execute('SELECT COUNT(*) FROM xlsx_files')
        file_count = cursor.fetchone()[0]
        cursor.execute('SELECT COUNT(*) FROM sheets')
        sheet_count = cursor.fetchone()[0]
        return {'file_count': file_count, 'sheet_count': sheet_count}
