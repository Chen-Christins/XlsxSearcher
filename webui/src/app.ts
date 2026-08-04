import {
  apiGet,
  apiPost,
  buildSearchQuery,
  type AppStatePayload,
  type Hit,
  type PreviewResponse,
  type SearchResponse,
  type SearchResult,
} from "./api.js";

declare global {
  interface Window {
    pywebview?: {
      api?: {
        choose_directory?: () => Promise<string | null>;
        choose_alias_file?: () => Promise<string | null>;
        choose_export_file?: () => Promise<string | null>;
        open_web_ui?: () => Promise<boolean>;
      };
    };
  }
}

const ROW_HEIGHT = 38;

function element<T extends HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `...${value.slice(-maxLength)}`;
}

function columnLetter(index: number): string {
  let value = index;
  let result = "";
  while (value > 0) {
    const remainder = (value - 1) % 26;
    result = String.fromCharCode(65 + remainder) + result;
    value = Math.floor((value - 1) / 26);
  }
  return result;
}

interface DropdownOption {
  label: string;
  value: string;
}

interface DropdownHandle {
  readonly value: string;
  setValue(value: string): void;
}

function createDropdown(
  slotId: string,
  options: DropdownOption[],
  initialValue: string,
  onChange: (value: string) => void,
): DropdownHandle {
  const slot = element(slotId);
  const root = document.createElement("div");
  root.className = "dropdown";

  const button = document.createElement("button");
  button.type = "button";
  button.className = "dropdown-btn";
  button.innerHTML = `
    <span class="dropdown-label"></span>
    <svg class="caret"><use href="#icon-chevron"/></svg>
  `;

  const menu = document.createElement("div");
  menu.className = "dropdown-menu";
  menu.hidden = true;

  let currentValue = initialValue;

  function renderLabel() {
    const option = options.find((item) => item.value === currentValue);
    const label = button.querySelector<HTMLElement>(".dropdown-label")!;
    label.textContent = option?.label ?? currentValue;
  }

  function closeMenu() {
    root.classList.remove("open");
    menu.hidden = true;
  }

  options.forEach((option) => {
    const item = document.createElement("button");
    item.type = "button";
    item.className = "dropdown-item";
    if (option.value === initialValue) item.classList.add("selected");
    item.dataset.value = option.value;
    item.innerHTML = `
      <span>${escapeHtml(option.label)}</span>
      <svg class="icon check"><use href="#icon-check"/></svg>
    `;
    item.addEventListener("click", () => {
      currentValue = option.value;
      options.forEach((entry) => {
        const itemElement = menu.querySelector<HTMLElement>(
          `[data-value="${entry.value}"]`,
        );
        itemElement?.classList.toggle("selected", entry.value === currentValue);
      });
      renderLabel();
      closeMenu();
      onChange(currentValue);
    });
    menu.appendChild(item);
  });

  button.addEventListener("click", () => {
    const willOpen = menu.hidden;
    closeMenu();
    if (willOpen) {
      root.classList.add("open");
      menu.hidden = false;
    }
  });

  document.addEventListener("click", (event) => {
    if (!root.contains(event.target as Node)) closeMenu();
  });

  root.appendChild(button);
  root.appendChild(menu);
  slot.replaceChildren(root);
  renderLabel();

  return {
    get value() {
      return currentValue;
    },
    setValue(value: string) {
      currentValue = value;
      options.forEach((entry) => {
        const itemElement = menu.querySelector<HTMLElement>(
          `[data-value="${entry.value}"]`,
        );
        itemElement?.classList.toggle("selected", entry.value === currentValue);
      });
      renderLabel();
    },
  };
}

interface Row {
  key: string;
  type: "file" | "sheet";
  filepath: string;
  filename: string;
  sheetName: string;
  aliases: string;
  path: string;
  count: string;
  indent: number;
  hasChildren: boolean;
  expanded: boolean;
}

interface UiState {
  results: SearchResult[];
  rows: Row[];
  expanded: Set<string>;
  viewMode: "grouped" | "flat";
  selectedRowKey: string;
  selectedFile: string;
  selectedSheet: string;
  previewHits: Hit[];
  previewIndex: number;
  polling: boolean;
}

const ui: UiState = {
  results: [],
  rows: [],
  expanded: new Set(),
  viewMode: "grouped",
  selectedRowKey: "",
  selectedFile: "",
  selectedSheet: "",
  previewHits: [],
  previewIndex: -1,
  polling: false,
};

let matchDropdown: DropdownHandle;
let sortDropdown: DropdownHandle;
let searchSequence = 0;
let previewSequence = 0;
let pendingPreviewIndex: number | null = null;
let renderedStartIndex = -1;

function findResult(filepath: string): SearchResult | undefined {
  return ui.results.find((result) => result.filepath === filepath);
}

function buildRows(): Row[] {
  const rows: Row[] = [];
  for (const result of ui.results) {
    const hasChildren = result.sheet_names.length > 0;
    const expanded = ui.expanded.has(result.filepath);
    rows.push({
      key: result.filepath,
      type: "file",
      filepath: result.filepath,
      filename: result.filename,
      sheetName: "",
      aliases: "",
      path: result.filepath,
      count: String(result.sheet_count),
      indent: 0,
      hasChildren,
      expanded: ui.viewMode === "grouped" && hasChildren && expanded,
    });

    if (ui.viewMode === "grouped" && expanded && hasChildren) {
      for (const sheetName of result.sheet_names) {
        rows.push({
          key: `${result.filepath}::${sheetName}`,
          type: "sheet",
          filepath: result.filepath,
          filename: result.filename,
          sheetName,
          aliases: (result.sheet_aliases[sheetName] ?? []).join(", "),
          path: result.filepath,
          count: "",
          indent: 1,
          hasChildren: false,
          expanded: false,
        });
      }
    } else if (ui.viewMode === "flat") {
      if (hasChildren) {
        for (const sheetName of result.sheet_names) {
          rows.push({
            key: `${result.filepath}::${sheetName}`,
            type: "sheet",
            filepath: result.filepath,
            filename: result.filename,
            sheetName,
            aliases: (result.sheet_aliases[sheetName] ?? []).join(", "),
            path: result.filepath,
            count: "",
            indent: 0,
            hasChildren: false,
            expanded: false,
          });
        }
      } else {
        rows.push({
          key: result.filepath,
          type: "file",
          filepath: result.filepath,
          filename: result.filename,
          sheetName: "",
          aliases: "",
          path: result.filepath,
          count: String(result.sheet_count),
          indent: 0,
          hasChildren: false,
          expanded: false,
        });
      }
    }
  }
  return rows;
}

function rowDisplayName(row: Row): string {
  return row.type === "file" ? row.filename : row.sheetName;
}

function renderResults() {
  ui.rows = buildRows();
  const body = element<HTMLElement>("results-body");
  const previousScrollTop = body.scrollTop;
  body.replaceChildren();

  const total = ui.rows.length;
  const spacer = document.createElement("div");
  spacer.style.position = "relative";
  spacer.style.height = `${total * ROW_HEIGHT}px`;
  body.appendChild(spacer);

  const viewportHeight = body.clientHeight || 400;
  const start = Math.max(0, Math.floor(previousScrollTop / ROW_HEIGHT) - 4);
  const end = Math.min(
    total,
    Math.ceil((previousScrollTop + viewportHeight) / ROW_HEIGHT) + 4,
  );
  renderedStartIndex = start;

  for (let index = start; index < end; index += 1) {
    const row = ui.rows[index];
    const rowElement = document.createElement("div");
    rowElement.className = `result-row ${row.type}-row`;
    if (row.key === ui.selectedRowKey) rowElement.classList.add("selected");
    rowElement.dataset.index = String(index);
    rowElement.style.top = `${index * ROW_HEIGHT}px`;
    rowElement.style.height = `${ROW_HEIGHT}px`;

    const chevron = row.hasChildren
      ? `<svg class="row-chevron ${row.expanded ? "expanded" : ""}"><use href="#icon-chevron"/></svg>`
      : `<svg class="row-chevron placeholder"><use href="#icon-chevron"/></svg>`;
    const icon = row.type === "file" ? "icon-file" : "icon-table";
    const paddingLeft = 12 + row.indent * 22;

    rowElement.innerHTML = `
      <div class="col-name" style="padding-left:${paddingLeft}px">
        ${chevron}
        <svg class="row-icon"><use href="#${icon}"/></svg>
        <span class="text" title="${escapeHtml(rowDisplayName(row))}">${escapeHtml(rowDisplayName(row))}</span>
      </div>
      <div class="col-count">${escapeHtml(row.count)}</div>
      <div class="col-alias" title="${escapeHtml(row.aliases)}">${escapeHtml(row.aliases)}</div>
      <div class="col-path" title="${escapeHtml(row.path)}">${escapeHtml(truncate(row.path, 72))}</div>
    `;
    spacer.appendChild(rowElement);
  }

  body.scrollTop = previousScrollTop;
  element<HTMLElement>("result-summary").textContent =
    `${ui.results.length} 个文件 / ${ui.results.reduce(
      (sum, result) => sum + result.sheet_count,
      0,
    )} 个子表`;
}

function currentSearchParams() {
  return {
    sheet: element<HTMLInputElement>("sheet-input").value.trim(),
    filename: element<HTMLInputElement>("filename-input").value.trim(),
    cell: element<HTMLInputElement>("cell-input").value.trim(),
    matchMode: matchDropdown.value,
    sortMode: sortDropdown.value,
  };
}

function scheduleSearch() {
  window.setTimeout(() => {
    void runSearch();
  }, 250);
}

async function runSearch() {
  const sequence = ++searchSequence;
  setStatus("正在搜索...");
  try {
    const response = await apiGet<SearchResponse>(
      buildSearchQuery(currentSearchParams()),
    );
    if (sequence !== searchSequence) return;
    ui.results = response.results;
    ui.expanded = new Set(
      response.results
        .filter((result) => result.sheet_count > 0)
        .map((result) => result.filepath),
    );
    renderResults();
    if (ui.results.length > 0) {
      setStatus(
        `找到 ${response.total_files} 个文件，${response.total_sheets} 个子表`,
      );
    } else {
      setStatus("没有匹配结果");
    }
  } catch (error) {
    if (sequence === searchSequence) {
      setStatus(`搜索失败：${messageOf(error)}`);
      toast(`搜索失败：${messageOf(error)}`, "error");
    }
  }
}

function setStatus(text: string) {
  element<HTMLElement>("status-text").textContent = text;
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function previewKeyword(): string {
  return (
    element<HTMLInputElement>("preview-input").value.trim() ||
    element<HTMLInputElement>("cell-input").value.trim() ||
    ""
  );
}

async function loadPreview(
  filepath: string,
  sheetName: string,
  startRow?: number,
  startCol?: number,
  navigateIndex: number | null = null,
) {
  if (!filepath || !sheetName) return;
  const sequence = ++previewSequence;
  pendingPreviewIndex = navigateIndex;
  ui.selectedFile = filepath;
  ui.selectedSheet = sheetName;

  const query = new URLSearchParams({
    filepath,
    sheet_name: sheetName,
    match_mode: matchDropdown.value,
  });
  const keyword = previewKeyword();
  if (keyword) query.set("keyword", keyword);
  if (startRow != null) query.set("start_row", String(startRow));
  if (startCol != null) query.set("start_col", String(startCol));

  setStatus("正在加载预览...");
  try {
    const response = await apiGet<PreviewResponse>(`/api/preview?${query}`);
    if (sequence !== previewSequence) return;
    ui.previewHits = response.hits;
    if (navigateIndex != null && response.hits.length > 0) {
      ui.previewIndex =
        ((navigateIndex % response.hits.length) + response.hits.length) %
        response.hits.length;
    } else {
      ui.previewIndex = response.hits.length > 0 ? 0 : -1;
    }
    renderPreview(response);
    setStatus(
      response.hits.length > 0
        ? `预览已加载，命中 ${response.hits.length} 处`
        : "预览已加载",
    );
  } catch (error) {
    if (sequence === previewSequence) {
      setStatus(`预览加载失败：${messageOf(error)}`);
      toast(`预览加载失败：${messageOf(error)}`, "error");
    }
  }
}

function renderPreview(response: PreviewResponse) {
  const title = element<HTMLElement>("preview-title");
  title.textContent = `${ui.selectedSheet} · ${truncate(ui.selectedFile, 72)}`;

  const table = element<HTMLTableElement>("preview-table");
  const empty = element<HTMLElement>("preview-empty");
  element<HTMLElement>("preview-scroll").scrollTop = 0;
  const data = response.data;
  table.hidden = data.length === 0;
  empty.hidden = data.length > 0;

  if (data.length === 0) {
    empty.textContent = `预览: ${ui.selectedSheet} (空表)`;
    updatePreviewControls();
    return;
  }

  const columnCount = Math.max(...data.map((row) => row.length), 0);
  table.replaceChildren();

  const head = document.createElement("thead");
  const headerRow = document.createElement("tr");
  const corner = document.createElement("th");
  corner.className = "row-number";
  corner.textContent = "";
  headerRow.appendChild(corner);
  for (let col = 0; col < columnCount; col += 1) {
    const headerCell = document.createElement("th");
    const label =
      response.header_row[col]?.trim() || columnLetter(response.start_col + col);
    headerCell.textContent = label;
    headerCell.title = label;
    headerRow.appendChild(headerCell);
  }
  head.appendChild(headerRow);

  const body = document.createElement("tbody");
  const hitPositions = new Set(
    response.hits.map((hit) => `${hit.row},${hit.col}`),
  );
  const currentHit =
    ui.previewIndex >= 0 ? ui.previewHits[ui.previewIndex] : null;

  data.forEach((rowData, rowOffset) => {
    const excelRow = response.start_row + rowOffset;
    const rowElement = document.createElement("tr");
    const rowNumber = document.createElement("td");
    rowNumber.className = "row-number";
    rowNumber.textContent = String(excelRow);
    rowElement.appendChild(rowNumber);

    for (let colOffset = 0; colOffset < columnCount; colOffset += 1) {
      const cell = document.createElement("td");
      cell.textContent = rowData[colOffset] ?? "";
      cell.title = cell.textContent;
      const excelCol = response.start_col + colOffset;
      if (hitPositions.has(`${excelRow},${excelCol}`)) {
        cell.classList.add("hit");
      }
      if (
        currentHit &&
        currentHit.row === excelRow &&
        currentHit.col === excelCol
      ) {
        cell.classList.add("current-hit");
      }
      rowElement.appendChild(cell);
    }
    body.appendChild(rowElement);
  });

  table.appendChild(head);
  table.appendChild(body);
  updatePreviewControls();
}

function updatePreviewControls() {
  const hasSelection = Boolean(ui.selectedFile && ui.selectedSheet);
  const hitCount = ui.previewHits.length;
  element<HTMLInputElement>("preview-input").disabled = !hasSelection;
  element<HTMLButtonElement>("preview-prev-btn").disabled = hitCount <= 1;
  element<HTMLButtonElement>("preview-next-btn").disabled = hitCount <= 1;
  element<HTMLButtonElement>("open-btn").disabled = !hasSelection;
  element<HTMLButtonElement>("locate-btn").disabled = !hasSelection;
  element<HTMLButtonElement>("copy-btn").disabled = !hasSelection;
  element<HTMLElement>("hit-label").textContent =
    hitCount > 0 && ui.previewIndex >= 0
      ? `命中 ${ui.previewIndex + 1}/${hitCount}`
      : `命中 ${hitCount}`;
}

function togglePreview() {
  const app = element<HTMLElement>("app");
  app.classList.toggle("preview-collapsed");
  const collapsed = app.classList.contains("preview-collapsed");
  const button = element<HTMLButtonElement>("toggle-preview-btn");
  button.title = collapsed ? "展开预览" : "折叠预览";
}

function navigatePreview(step: number) {
  if (ui.previewHits.length === 0) return;
  const nextIndex =
    ((ui.previewIndex + step) % ui.previewHits.length + ui.previewHits.length) %
    ui.previewHits.length;
  const hit = ui.previewHits[nextIndex];
  void loadPreview(
    ui.selectedFile,
    ui.selectedSheet,
    Math.max(hit.row - 3, 1),
    Math.max(hit.col - 2, 1),
    nextIndex,
  );
}

function updateAppState(state: AppStatePayload) {
  element<HTMLElement>("version-pill").textContent = `v${state.version}`;
  element<HTMLElement>("dir-path").textContent =
    state.directory || "未选择目录";
  element<HTMLElement>("dir-path").title = state.directory;
  element<HTMLElement>("index-stats").textContent =
    `${state.index.file_count} 文件 · ${state.index.sheet_count} 子表 · ` +
    `${state.index.indexed_cell_sheet_count} 已深度索引`;

  const running = state.job.running;
  element<HTMLButtonElement>("scan-btn").disabled = running;
  element<HTMLButtonElement>("deep-index-btn").disabled = running;
  element<HTMLButtonElement>("clear-index-btn").disabled = running;

  const progress = element<HTMLElement>("job-progress");
  progress.hidden = !running;
  if (running) {
    const percent =
      state.job.total > 0
        ? Math.round((state.job.current / state.job.total) * 100)
        : 0;
    element<HTMLElement>("progress-fill").style.width = `${percent}%`;
    element<HTMLElement>("progress-text").textContent =
      state.job.total > 0
        ? `${state.job.current}/${state.job.total}`
        : "处理中...";
    setStatus(state.job.message || "任务执行中...");
  }
}

async function refreshState() {
  try {
    const state = await apiGet<AppStatePayload>("/api/state");
    updateAppState(state);
    if (state.job.running && !ui.polling) {
      ui.polling = true;
      void pollJob();
    }
  } catch (error) {
    setStatus(`状态刷新失败：${messageOf(error)}`);
  }
}

async function pollJob() {
  while (ui.polling) {
    await delay(500);
    try {
      const state = await apiGet<AppStatePayload>("/api/state");
      updateAppState(state);
      if (!state.job.running) {
        ui.polling = false;
        if (state.job.error) {
          setStatus(`任务失败：${state.job.error}`);
          toast(`任务失败：${state.job.error}`, "error");
        } else {
          setStatus(state.job.message || "任务完成");
          toast(state.job.message || "任务完成", "success");
        }
        await runSearch();
        await refreshState();
        return;
      }
    } catch (error) {
      setStatus(`任务状态刷新失败：${messageOf(error)}`);
    }
  }
}

async function startScan(directory: string) {
  if (!directory) return;
  try {
    await apiPost("/api/scan", { directory });
    setStatus("扫描已开始...");
    await refreshState();
  } catch (error) {
    setStatus(`扫描启动失败：${messageOf(error)}`);
    toast(`扫描启动失败：${messageOf(error)}`, "error");
  }
}

async function pickDirectory() {
  const bridge = window.pywebview?.api;
  if (bridge?.choose_directory) {
    try {
      const directory = await bridge.choose_directory();
      if (directory) await startScan(directory);
    } catch (error) {
      toast(`选择目录失败：${messageOf(error)}`, "error");
    }
    return;
  }

  const input = document.createElement("input") as HTMLInputElement & {
    webkitdirectory: boolean;
  };
  input.type = "file";
  input.webkitdirectory = true;
  input.addEventListener("change", async () => {
    const file = input.files?.[0];
    if (!file) return;
    const rawPath = (file as File & { path?: string }).path;
    if (!rawPath) {
      toast("当前 WebView 未返回目录路径，请使用桌面客户端或手动输入路径", "error");
      return;
    }
    const directory = rawPath
      .slice(0, rawPath.length - file.webkitRelativePath.length)
      .replace(/[\\/]+$/, "");
    if (directory) await startScan(directory);
  });
  input.click();
}

async function startDeepIndex() {
  try {
    await apiPost("/api/deep-index", {});
    setStatus("深度索引已启动...");
    await refreshState();
  } catch (error) {
    setStatus(`深度索引启动失败：${messageOf(error)}`);
    toast(`深度索引启动失败：${messageOf(error)}`, "error");
  }
}

async function clearIndex() {
  if (!window.confirm("确定要清空所有索引数据吗？")) return;
  try {
    await apiPost("/api/clear-index", {});
    toast("索引已清空", "success");
    await refreshState();
    await runSearch();
  } catch (error) {
    toast(`清空索引失败：${messageOf(error)}`, "error");
  }
}

async function readFileText(file: File): Promise<string> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return new TextDecoder("gbk").decode(bytes);
  }
}

async function importAliases() {
  const bridge = window.pywebview?.api;
  if (bridge?.choose_alias_file) {
    try {
      const path = await bridge.choose_alias_file();
      if (!path) return;
      const response = await apiPost<{ imported: number }>(
        "/api/import-aliases-file",
        { path },
      );
      toast(`导入成功：${response.imported} 条映射`, "success");
      await refreshState();
      await runSearch();
    } catch (error) {
      toast(`导入映射失败：${messageOf(error)}`, "error");
    }
    return;
  }

  const input = document.createElement("input");
  input.type = "file";
  input.accept = ".bat,.txt,.cmd";
  input.addEventListener("change", async () => {
    const file = input.files?.[0];
    if (!file) return;
    try {
      const content = await readFileText(file);
      const response = await apiPost<{ imported: number }>(
        "/api/import-aliases",
        { source_name: file.name, content },
      );
      toast(`导入成功：${response.imported} 条映射`, "success");
      await refreshState();
      await runSearch();
    } catch (error) {
      toast(`导入映射失败：${messageOf(error)}`, "error");
    }
  });
  input.click();
}

function buildCsv(): string {
  const lines = ["文件名,子表名称,别名,文件路径,命中子表数"];
  for (const result of ui.results) {
    if (result.sheet_names.length > 0) {
      for (const sheetName of result.sheet_names) {
        lines.push(
          [
            result.filename,
            sheetName,
            (result.sheet_aliases[sheetName] ?? []).join("; "),
            result.filepath,
            result.sheet_count,
          ]
            .map((value) => `"${String(value).replaceAll('"', '""')}"`)
            .join(","),
        );
      }
    } else {
      lines.push(
        [result.filename, "", "", result.filepath, result.sheet_count]
          .map((value) => `"${String(value).replaceAll('"', '""')}"`)
          .join(","),
      );
    }
  }
  return lines.join("\n");
}

function downloadCsv() {
  const blob = new Blob(["\ufeff", buildCsv()], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = "xlsx_search_results.csv";
  anchor.click();
  URL.revokeObjectURL(url);
}

async function exportResults() {
  const bridge = window.pywebview?.api;
  if (bridge?.choose_export_file) {
    try {
      const path = await bridge.choose_export_file();
      if (!path) return;
      const params = currentSearchParams();
      await apiPost("/api/export", { path, ...params });
      toast(`已导出到 ${path}`, "success");
    } catch (error) {
      toast(`导出失败：${messageOf(error)}`, "error");
    }
    return;
  }
  downloadCsv();
  toast("已导出 CSV", "success");
}

async function runAction(action: "open" | "locate" | "copy") {
  if (!ui.selectedFile) {
    toast("请先选择文件", "error");
    return;
  }
  try {
    await apiPost("/api/action", {
      action,
      filepath: ui.selectedFile,
    });
    if (action === "copy") toast("路径已复制到剪贴板", "success");
  } catch (error) {
    toast(`${action === "open" ? "打开" : action === "locate" ? "定位" : "复制"}失败：${messageOf(error)}`, "error");
  }
}

async function openWebUi() {
  const bridge = window.pywebview?.api;
  if (bridge?.open_web_ui) {
    try {
      await bridge.open_web_ui();
      toast("已在系统浏览器打开 Web 界面", "success");
    } catch (error) {
      toast(`打开 Web 界面失败：${messageOf(error)}`, "error");
    }
    return;
  }
  window.open(location.origin, "_blank");
}

function toast(message: string, type: "success" | "error" | "info" = "info") {
  const container = element<HTMLElement>("toast-container");
  const item = document.createElement("div");
  item.className = `toast ${type}`;
  const iconName =
    type === "success" ? "icon-check" : type === "error" ? "icon-trash" : "icon-search";
  item.innerHTML = `<svg class="icon"><use href="#${iconName}"/></svg><span></span>`;
  item.querySelector("span")!.textContent = message;
  container.appendChild(item);
  window.setTimeout(() => item.remove(), 2800);
}

function handleRowClick(indexText: string) {
  const index = Number(indexText);
  const row = ui.rows[index];
  if (!row) return;
  ui.selectedRowKey = row.key;
  renderResults();

  if (row.type === "file" && row.hasChildren) {
    if (ui.expanded.has(row.filepath)) {
      ui.expanded.delete(row.filepath);
    } else {
      ui.expanded.add(row.filepath);
    }
    renderResults();
  }

  const result = findResult(row.filepath);
  const sheetName =
    row.type === "sheet" ? row.sheetName : (result?.sheet_names[0] ?? "");
  if (sheetName) {
    void loadPreview(row.filepath, sheetName);
  } else {
    ui.selectedFile = row.filepath;
    ui.selectedSheet = "";
    element<HTMLElement>("preview-title").textContent = "预览";
    element<HTMLElement>("preview-empty").textContent = "该文件没有可预览的子表";
    element<HTMLElement>("preview-table").hidden = true;
    element<HTMLElement>("preview-empty").hidden = false;
    updatePreviewControls();
  }
}

function init() {
  matchDropdown = createDropdown(
    "match-dropdown",
    [
      { label: "模糊匹配", value: "fuzzy" },
      { label: "前缀匹配", value: "prefix" },
      { label: "精确匹配", value: "exact" },
    ],
    "fuzzy",
    () => scheduleSearch(),
  );

  sortDropdown = createDropdown(
    "sort-dropdown",
    [
      { label: "文件名 A-Z", value: "filename_asc" },
      { label: "文件名 Z-A", value: "filename_desc" },
      { label: "子表数最多", value: "sheet_count_desc" },
      { label: "子表数最少", value: "sheet_count_asc" },
    ],
    "filename_asc",
    () => {
      if (sortDropdown.value.startsWith("sheet_count")) {
        const sorted = [...ui.results].sort((a, b) => {
          const diff = a.sheet_count - b.sheet_count;
          if (diff !== 0) return sortDropdown.value.endsWith("desc") ? -diff : diff;
          return a.filename.localeCompare(b.filename);
        });
        ui.results = sorted;
        renderResults();
      } else {
        void runSearch();
      }
    },
  );

  element<HTMLInputElement>("sheet-input").addEventListener("input", scheduleSearch);
  element<HTMLInputElement>("filename-input").addEventListener("input", scheduleSearch);
  element<HTMLInputElement>("cell-input").addEventListener("input", scheduleSearch);

  document.querySelectorAll<HTMLButtonElement>(".segmented-btn").forEach((button) => {
    button.addEventListener("click", () => {
      const viewMode = button.dataset.view as "grouped" | "flat";
      ui.viewMode = viewMode;
      document.querySelectorAll<HTMLButtonElement>(".segmented-btn").forEach((entry) => {
        entry.classList.toggle("active", entry === button);
      });
      renderResults();
    });
  });

  element<HTMLButtonElement>("pick-dir-btn").addEventListener("click", () => {
    void pickDirectory();
  });
  element<HTMLButtonElement>("scan-btn").addEventListener("click", () => {
    const stateDirectory = element<HTMLElement>("dir-path").textContent;
    if (stateDirectory && stateDirectory !== "未选择目录") {
      void startScan(stateDirectory);
    } else {
      void pickDirectory();
    }
  });
  element<HTMLButtonElement>("deep-index-btn").addEventListener("click", () => {
    void startDeepIndex();
  });
  element<HTMLButtonElement>("clear-index-btn").addEventListener("click", () => {
    void clearIndex();
  });
  element<HTMLButtonElement>("import-alias-btn").addEventListener("click", () => {
    void importAliases();
  });
  element<HTMLButtonElement>("open-browser-btn").addEventListener("click", () => {
    void openWebUi();
  });
  element<HTMLButtonElement>("export-btn").addEventListener("click", () => {
    void exportResults();
  });

  element<HTMLButtonElement>("toggle-preview-btn").addEventListener("click", togglePreview);
  element<HTMLInputElement>("preview-input").addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    if (ui.selectedFile && ui.selectedSheet) {
      void loadPreview(ui.selectedFile, ui.selectedSheet);
    }
  });
  element<HTMLButtonElement>("preview-prev-btn").addEventListener("click", () => {
    navigatePreview(-1);
  });
  element<HTMLButtonElement>("preview-next-btn").addEventListener("click", () => {
    navigatePreview(1);
  });
  element<HTMLButtonElement>("open-btn").addEventListener("click", () => {
    void runAction("open");
  });
  element<HTMLButtonElement>("locate-btn").addEventListener("click", () => {
    void runAction("locate");
  });
  element<HTMLButtonElement>("copy-btn").addEventListener("click", () => {
    void runAction("copy");
  });

  const resultsBody = element<HTMLElement>("results-body");
  resultsBody.addEventListener("click", (event) => {
    const rowElement = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-index]",
    );
    if (rowElement?.dataset.index != null) handleRowClick(rowElement.dataset.index);
  });
  resultsBody.addEventListener("dblclick", (event) => {
    const rowElement = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-index]",
    );
    if (rowElement?.dataset.index != null) {
      const row = ui.rows[Number(rowElement.dataset.index)];
      if (row) void runAction("open");
    }
  });
  resultsBody.addEventListener("scroll", () => {
    const start = Math.floor(resultsBody.scrollTop / ROW_HEIGHT);
    if (start !== renderedStartIndex) renderResults();
  });

  document.addEventListener("keydown", (event) => {
    const isShortcutModifier = event.ctrlKey || event.metaKey;
    if (isShortcutModifier && event.code === "Backquote") {
      event.preventDefault();
      togglePreview();
    }
  });

  void refreshState();
  void runSearch();
}

document.addEventListener("DOMContentLoaded", init);
