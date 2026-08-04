export interface IndexStatus {
  file_count: number;
  sheet_count: number;
  indexed_cell_sheet_count: number;
  pending_deep_index_count: number;
}

export interface AliasStats {
  alias_count: number;
  mapping_count: number;
}

export interface JobState {
  running: boolean;
  kind: string;
  current: number;
  total: number;
  message: string;
  result: Record<string, unknown> | null;
  error: string | null;
}

export interface AppStatePayload {
  version: string;
  directory: string;
  index: IndexStatus;
  aliases: AliasStats;
  job: JobState;
}

export interface SearchResult {
  filename: string;
  filepath: string;
  sheet_names: string[];
  sheet_count: number;
  sheet_names_display: string;
  sheet_aliases: Record<string, string[]>;
}

export interface SearchResponse {
  results: SearchResult[];
  total_files: number;
  total_sheets: number;
}

export interface Hit {
  row: number;
  col: number;
  value: string;
}

export interface PreviewResponse {
  hits: Hit[];
  data: string[][];
  header_row: string[];
  start_row: number;
  start_col: number;
}

interface ApiEnvelope<T> {
  ok: boolean;
  error?: string;
  [key: string]: unknown;
}

async function parseResponse<T>(response: Response): Promise<T> {
  const payload = (await response.json()) as ApiEnvelope<T>;
  if (!response.ok || payload.ok === false) {
    throw new Error(payload.error || `请求失败 (${response.status})`);
  }
  return payload as T;
}

export async function apiGet<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: { "Accept": "application/json" },
  });
  return parseResponse<T>(response);
}

export async function apiPost<T>(path: string, body: unknown): Promise<T> {
  const response = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return parseResponse<T>(response);
}

export function buildSearchQuery(params: {
  sheet: string;
  filename: string;
  cell: string;
  matchMode: string;
  sortMode: string;
}): string {
  const query = new URLSearchParams({
    sheet: params.sheet,
    filename: params.filename,
    cell: params.cell,
    match_mode: params.matchMode,
    sort_mode: params.sortMode,
  });
  return `/api/search?${query.toString()}`;
}
