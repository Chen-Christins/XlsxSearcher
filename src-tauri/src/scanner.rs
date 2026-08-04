use crate::types::{Hit, PreviewResponse};
use calamine::{open_workbook_auto, Data, DataType, Reader};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;
use regex::bytes::Regex;
use std::borrow::Cow;
use std::fs;
use std::io::Read;
use std::path::Path;
use walkdir::WalkDir;
use zip::ZipArchive;

const SUPPORTED_EXTENSIONS: [&str; 3] = [".xlsx", ".xlsm", ".xls"];
const MAX_DEEP_INDEX_FILE_MB: u64 = 80;

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let ext = format!(".{}", ext.to_lowercase());
            SUPPORTED_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

pub fn walk_files(directory: &str) -> Vec<(String, String, f64)> {
    let mut files = Vec::new();
    for entry in WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !entry.file_type().is_dir() || !name.starts_with('.')
        })
    {
        if let Ok(entry) = entry {
            if entry.file_type().is_file() && is_supported(entry.path()) {
                if let Ok(metadata) = fs::metadata(entry.path()) {
                    if let Ok(duration) = metadata.modified().and_then(|time| {
                        time.duration_since(std::time::UNIX_EPOCH)
                            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
                    }) {
                        let mtime = duration.as_secs_f64();
                        let filepath = entry.path().to_string_lossy().to_string();
                        let filename = entry.file_name().to_string_lossy().to_string();
                        files.push((filepath, filename, mtime));
                    }
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

pub fn sheet_names(filepath: &str) -> Vec<String> {
    let path = Path::new(filepath);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if ext == "xls" {
        return sheet_names_calamine(filepath);
    }

    if let Ok(names) = sheet_names_fast_zip(filepath) {
        if !names.is_empty() {
            return names;
        }
    }
    sheet_names_calamine(filepath)
}

fn sheet_names_fast_zip(filepath: &str) -> Result<Vec<String>, String> {
    let file = fs::File::open(filepath).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    archive
        .by_name("xl/workbook.xml")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    let regex = Regex::new(r#"<sheet\s[^>]*?name="([^"]*)"#).map_err(|e| e.to_string())?;
    let names = regex
        .captures_iter(&raw)
        .filter_map(|capture| capture.get(1))
        .filter_map(|m| String::from_utf8(m.as_bytes().to_vec()).ok())
        .collect();
    Ok(names)
}

fn sheet_names_calamine(filepath: &str) -> Vec<String> {
    match open_workbook_auto(filepath) {
        Ok(workbook) => workbook.sheet_names().to_vec(),
        Err(_) => Vec::new(),
    }
}

fn data_to_string(data: &Data) -> String {
    match data {
        Data::String(value) => value.clone(),
        Data::Float(value) => value.to_string(),
        Data::Int(value) => value.to_string(),
        Data::Bool(value) => value.to_string(),
        Data::DateTime(value) => value.to_string(),
        Data::DateTimeIso(value) | Data::DurationIso(value) => value.clone(),
        _ => String::new(),
    }
}

fn trim_row(row: &[String]) -> Vec<String> {
    let mut last_non_empty = None;
    for (index, value) in row.iter().enumerate() {
        if !value.is_empty() {
            last_non_empty = Some(index);
        }
    }
    match last_non_empty {
        Some(index) => row[..=index].to_vec(),
        None => Vec::new(),
    }
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let needle_bytes = needle.as_bytes();
    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }
    for i in 0..=(haystack_bytes.len() - needle_bytes.len()) {
        if haystack_bytes[i..i + needle_bytes.len()].eq_ignore_ascii_case(needle_bytes) {
            return true;
        }
    }
    false
}

fn cell_matches(value: &str, keyword: &str, mode: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }
    match mode {
        "exact" => value.eq_ignore_ascii_case(keyword),
        "prefix" => {
            let bytes = keyword.len();
            value.len() >= bytes
                && value.is_char_boundary(bytes)
                && value[..bytes].eq_ignore_ascii_case(keyword)
        }
        _ => contains_ignore_case(value, keyword),
    }
}

pub fn extract_cell_texts(
    filepath: &str,
    sheet_names: &[String],
    max_chars_per_sheet: usize,
) -> Result<Vec<String>, String> {
    let metadata = fs::metadata(filepath).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_DEEP_INDEX_FILE_MB * 1024 * 1024 {
        return Ok(vec![String::new(); sheet_names.len()]);
    }

    let mut workbook = open_workbook_auto(filepath).map_err(|e| e.to_string())?;
    let mut results = Vec::with_capacity(sheet_names.len());
    for sheet_name in sheet_names {
        let mut parts = Vec::new();
        let mut char_count = 0usize;
        if let Ok(range) = workbook.worksheet_range(sheet_name) {
            'outer: for row in range.rows() {
                for cell in row {
                    let value = data_to_string(cell);
                    if value.is_empty() {
                        continue;
                    }
                    char_count += value.chars().count() + 1;
                    parts.push(value);
                    if char_count > max_chars_per_sheet {
                        break 'outer;
                    }
                }
            }
        }
        results.push(parts.join(" "));
    }
    Ok(results)
}

#[allow(clippy::too_many_arguments)]
pub fn read_sheet_with_hits(
    filepath: &str,
    sheet_name: &str,
    keyword: Option<&str>,
    match_mode: &str,
    max_hits: usize,
    preview_rows: usize,
    preview_cols: usize,
    start_row: Option<usize>,
    start_col: Option<usize>,
) -> Result<PreviewResponse, String> {
    let ext = Path::new(filepath)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();
    if ext == "xlsx" || ext == "xlsm" {
        if let Ok(response) = read_sheet_with_hits_streaming(
            filepath,
            sheet_name,
            keyword,
            match_mode,
            max_hits,
            preview_rows,
            preview_cols,
            start_row,
            start_col,
        ) {
            return Ok(response);
        }
    }
    read_sheet_with_hits_calamine(
        filepath,
        sheet_name,
        keyword,
        match_mode,
        max_hits,
        preview_rows,
        preview_cols,
        start_row,
        start_col,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_sheet_with_hits_calamine(
    filepath: &str,
    sheet_name: &str,
    keyword: Option<&str>,
    match_mode: &str,
    max_hits: usize,
    preview_rows: usize,
    preview_cols: usize,
    start_row: Option<usize>,
    start_col: Option<usize>,
) -> Result<PreviewResponse, String> {
    let mut workbook = open_workbook_auto(filepath).map_err(|e| e.to_string())?;
    let range = workbook
        .worksheet_range(sheet_name)
        .map_err(|e| format!("sheet not found: {}", e))?;

    let header_row = range
        .rows()
        .next()
        .map(|row| {
            trim_row(
                &row.iter()
                    .take(preview_cols)
                    .map(data_to_string)
                    .collect::<Vec<String>>(),
            )
        })
        .unwrap_or_default();

    let mut hits = Vec::new();
    if let Some(keyword) = keyword {
        'hits: for (row_index, row) in range.rows().enumerate() {
            for (col_index, value) in row.iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let text = data_to_string(value);
                if cell_matches(&text, keyword, match_mode) {
                    hits.push(Hit {
                        row: row_index as i64 + 1,
                        col: col_index as i64 + 1,
                        value: text,
                    });
                    if hits.len() >= max_hits {
                        break 'hits;
                    }
                }
            }
        }
    }

    let (window_row, window_col) = preview_window(hits.first(), start_row, start_col);

    let data = range
        .rows()
        .skip(window_row.saturating_sub(1))
        .take(preview_rows)
        .map(|row| {
            let start = window_col.saturating_sub(1);
            let end = (start + preview_cols).min(row.len());
            trim_row(
                &row[start..end]
                    .iter()
                    .map(data_to_string)
                    .collect::<Vec<String>>(),
            )
        })
        .collect();

    Ok(PreviewResponse {
        hits,
        data,
        header_row,
        start_row: window_row,
        start_col: window_col,
    })
}

fn preview_window(
    first_hit: Option<&Hit>,
    start_row: Option<usize>,
    start_col: Option<usize>,
) -> (usize, usize) {
    if let Some(row) = start_row {
        (row, start_col.unwrap_or(1))
    } else if let Some(first_hit) = first_hit {
        (
            (first_hit.row as usize).saturating_sub(3).max(1),
            (first_hit.col as usize).saturating_sub(2).max(1),
        )
    } else {
        (2, 1)
    }
}

fn xml_attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes()
        .filter_map(Result::ok)
        .find(|a| a.key.local_name().as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn parse_shared_strings(xml: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut current = String::new();
    let mut in_si = false;
    while i < xml.len() {
        if xml[i] != b'<' {
            i += 1;
            continue;
        }
        if xml[i..].starts_with(b"<si/>") {
            out.push(String::new());
            i += 5;
        } else if xml[i..].starts_with(b"<si") {
            in_si = true;
            current.clear();
            i += 3;
        } else if xml[i..].starts_with(b"</si") {
            in_si = false;
            out.push(std::mem::take(&mut current));
            i += 4;
        } else if in_si && xml[i..].starts_with(b"<t") {
            i += 2;
            while i < xml.len() && xml[i] != b'>' {
                i += 1;
            }
            i += 1;
            let start = i;
            while i < xml.len() && !xml[i..].starts_with(b"</t") {
                i += 1;
            }
            current.push_str(&unescape(&xml[start..i]));
            i += 3;
        } else {
            while i < xml.len() && xml[i] != b'>' {
                i += 1;
            }
            i += 1;
        }
    }
    out
}

#[derive(PartialEq, Clone, Copy)]
enum CellKind {
    Shared,
    Inline,
    Str,
    Number,
}

fn col_from_ref_bytes(cell_ref: &[u8]) -> Option<usize> {
    let mut col = 0usize;
    let mut any = false;
    for &byte in cell_ref {
        if byte.is_ascii_alphabetic() {
            any = true;
            col = col * 26 + (byte.to_ascii_uppercase() - b'A' + 1) as usize;
        } else {
            break;
        }
    }
    if any {
        Some(col)
    } else {
        None
    }
}

fn find_byte(hay: &[u8], from: usize, needle: u8) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    memchr::memchr(needle, &hay[from..]).map(|p| from + p)
}

fn find_bytes(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from + needle.len() > hay.len() {
        return None;
    }
    let first = needle[0];
    let rest = &needle[1..];
    let mut i = from;
    loop {
        let Some(offset) = memchr::memchr(first, &hay[i..]) else {
            return None;
        };
        let at = i + offset;
        if hay[at + 1..].starts_with(rest) {
            return Some(at);
        }
        i = at + 1;
    }
}

/// Reads a quoted attribute value (`key="..."` or `key='...'`).
fn attr_value<'a>(tag: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut i = 0;
    while i + key.len() < tag.len() {
        if &tag[i..i + key.len()] == key && tag[i + key.len()] == b'=' {
            let q = i + key.len() + 1;
            if q < tag.len() && (tag[q] == b'"' || tag[q] == b'\'') {
                let quote = tag[q];
                let start = q + 1;
                if let Some(rel) = tag[start..].iter().position(|&b| b == quote) {
                    return Some(&tag[start..start + rel]);
                }
            }
        }
        i += 1;
    }
    None
}

/// Decodes XML entities into a String.
fn unescape(text: &[u8]) -> String {
    if !text.contains(&b'&') {
        return String::from_utf8_lossy(text).into_owned();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'&' {
            let end = text[i + 1..].iter().position(|&b| b == b';').map(|p| i + 1 + p);
            if let Some(end) = end {
                let entity = &text[i + 1..end];
                match entity {
                    b"amp" => out.push('&'),
                    b"lt" => out.push('<'),
                    b"gt" => out.push('>'),
                    b"quot" => out.push('"'),
                    b"apos" => out.push('\''),
                    _ => {
                        let code = entity.strip_prefix(b"#x").or_else(|| entity.strip_prefix(b"#X")).and_then(|h| {
                            std::str::from_utf8(h).ok().and_then(|s| u32::from_str_radix(s, 16).ok())
                        }).or_else(|| {
                            entity.strip_prefix(b"#").and_then(|d| std::str::from_utf8(d).ok()).and_then(|s| s.parse::<u32>().ok())
                        });
                        match code.and_then(char::from_u32) {
                            Some(c) => out.push(c),
                            None => out.push('&'),
                        }
                    }
                }
                i = end + 1;
                continue;
            }
        }
        out.push_str(&String::from_utf8_lossy(&text[i..]));
        break;
    }
    out
}

/// Returns the text between an opening tag and its matching closing tag,
/// along with the position just after the closing tag.
fn text_between<'a>(hay: &'a [u8], from: usize, open: &[u8], close: &[u8]) -> Option<(&'a [u8], usize)> {
    let os = find_bytes(hay, from, open)?;
    let oe = find_byte(hay, os, b'>')?;
    let start = oe + 1;
    let ce = find_bytes(hay, start, close)?;
    Some((&hay[start..ce], ce + close.len()))
}

/// Extracts a cell's value from its inner content, borrowing the bytes when
/// possible (no XML entities, single text run) to avoid per-cell allocation.
fn cell_value<'a>(content: &'a [u8], kind: CellKind, sst: &'a [String]) -> Cow<'a, str> {
    fn borrowed_or_unescaped(text: &[u8]) -> Cow<'_, str> {
        if !text.contains(&b'&') {
            Cow::Borrowed(std::str::from_utf8(text).unwrap_or(""))
        } else {
            Cow::Owned(unescape(text))
        }
    }
    match kind {
        CellKind::Inline => {
            let Some((text, next)) = text_between(content, 0, b"<t", b"</t") else {
                return Cow::Borrowed("");
            };
            if find_bytes(content, next, b"<t").is_none() {
                borrowed_or_unescaped(text)
            } else {
                let mut out = String::new();
                let mut pos = 0;
                while let Some((run, nxt)) = text_between(content, pos, b"<t", b"</t") {
                    out.push_str(&unescape(run));
                    pos = nxt;
                }
                Cow::Owned(out)
            }
        }
        CellKind::Shared => {
            let value = text_between(content, 0, b"<v", b"</v")
                .map(|(t, _)| t)
                .unwrap_or_default();
            let index = std::str::from_utf8(value)
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(usize::MAX);
            sst.get(index)
                .map(|s| Cow::Borrowed(s.as_str()))
                .unwrap_or_else(|| Cow::Borrowed(""))
        }
        _ => text_between(content, 0, b"<v", b"</v")
            .map(|(t, _)| borrowed_or_unescaped(t))
            .unwrap_or_else(|| Cow::Borrowed("")),
    }
}

/// Streams worksheet cells, calling `on_cell` with `(row, col, value)` (all
/// 1-based). Cells for which `include` returns `false` are skipped entirely,
/// so a preview window can be extracted without materializing the whole sheet.
/// Returns `Ok(false)` when `on_cell` signals a stop.
fn scan_sheet(
    xml: &[u8],
    sst: &[String],
    max_row: usize,
    include: &mut impl FnMut(usize, usize) -> bool,
    on_cell: &mut impl FnMut(usize, usize, Cow<'_, str>) -> bool,
) -> Result<bool, String> {
    let mut pos = 0usize;
    let mut row = 0usize;
    loop {
        let Some(row_start) = find_bytes(xml, pos, b"<row") else {
            break;
        };
        let next = xml.get(row_start + 4).copied().unwrap_or(b'>');
        if next.is_ascii_alphabetic() {
            pos = row_start + 4;
            continue;
        }
        let Some(row_end) = find_byte(xml, row_start, b'>') else {
            break;
        };
        let row_tag = &xml[row_start + 1..row_end];
        row = attr_value(row_tag, b"r")
            .and_then(|v| std::str::from_utf8(v).ok())
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(row + 1);
        if row > max_row {
            break;
        }
        if xml[row_end - 1] == b'/' {
            pos = row_end + 1;
            continue;
        }
        let Some(row_close) = find_bytes(xml, row_end + 1, b"</row") else {
            break;
        };

        if scan_row_cells(
            xml,
            row_end + 1,
            row_close,
            row,
            sst,
            include,
            on_cell,
        )? == false
        {
            return Ok(false);
        }
        pos = row_close;
    }
    Ok(true)
}

fn scan_row_cells(
    xml: &[u8],
    content_start: usize,
    row_close: usize,
    row: usize,
    sst: &[String],
    include: &mut impl FnMut(usize, usize) -> bool,
    on_cell: &mut impl FnMut(usize, usize, Cow<'_, str>) -> bool,
) -> Result<bool, String> {
    let mut i = content_start;
    let mut col_counter = 0usize;
    loop {
        while i < row_close && xml[i] != b'<' {
            i += 1;
        }
        if i >= row_close {
            return Ok(true);
        }
        if i + 1 >= row_close {
            return Ok(true);
        }
        if xml[i + 1] == b'/' {
            while i < row_close && xml[i] != b'>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if xml[i + 1] != b'c' {
            while i < row_close && xml[i] != b'>' {
                i += 1;
            }
            i += 1;
            continue;
        }
        let mut j = i + 2;
        while j < row_close && xml[j] != b'>' {
            j += 1;
        }
        if j >= row_close {
            return Ok(true);
        }
        let tag = &xml[i + 2..j];
        let col = attr_value(tag, b"r")
            .and_then(col_from_ref_bytes)
            .unwrap_or_else(|| {
                col_counter += 1;
                col_counter
            });
        let kind = match attr_value(tag, b"t") {
            Some(b"s") => CellKind::Shared,
            Some(b"inlineStr") => CellKind::Inline,
            Some(b"str") => CellKind::Str,
            _ => CellKind::Number,
        };
        let included = include(row, col);
        let self_closing = xml[j - 1] == b'/';
        let content = j + 1;
        if self_closing {
            i = content;
            continue;
        }
        let mut k = content;
        while k + 3 <= row_close
            && !(xml[k] == b'<' && xml[k + 1] == b'/' && xml[k + 2] == b'c')
        {
            k += 1;
        }
        let cell_close = if k + 3 <= row_close { k } else { row_close };
        if included {
            let value = cell_value(&xml[content..cell_close], kind, sst);
            if !value.is_empty() && !on_cell(row, col, value) {
                return Ok(false);
            }
        }
        let mut m = cell_close;
        while m < row_close && xml[m] != b'>' {
            m += 1;
        }
        i = m + 1;
    }
}

fn workbook_sheet_r_id(workbook_xml: &[u8], sheet_name: &str) -> Option<String> {
    let mut reader = XmlReader::from_reader(workbook_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if e.local_name().as_ref() == b"sheet" =>
            {
                let name = xml_attr(&e, b"name")?;
                if name == sheet_name {
                    return xml_attr(&e, b"id");
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

fn rels_target(rels_xml: &[u8], r_id: &str) -> Option<String> {
    let mut reader = XmlReader::from_reader(rels_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e))
                if e.local_name().as_ref() == b"Relationship" =>
            {
                if xml_attr(&e, b"Id").as_deref() == Some(r_id) {
                    return xml_attr(&e, b"Target");
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    None
}

fn rid_to_sheet_index(r_id: &str) -> usize {
    r_id.trim_start_matches(|c: char| !c.is_ascii_digit())
        .parse::<usize>()
        .unwrap_or(1)
}

#[allow(clippy::too_many_arguments)]
fn read_sheet_with_hits_streaming(
    filepath: &str,
    sheet_name: &str,
    keyword: Option<&str>,
    match_mode: &str,
    max_hits: usize,
    preview_rows: usize,
    preview_cols: usize,
    start_row: Option<usize>,
    start_col: Option<usize>,
) -> Result<PreviewResponse, String> {
    let file = fs::File::open(filepath).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let mut workbook_xml = Vec::new();
    archive
        .by_name("xl/workbook.xml")
        .map_err(|e| e.to_string())?
        .read_to_end(&mut workbook_xml)
        .map_err(|e| e.to_string())?;
    let r_id = workbook_sheet_r_id(&workbook_xml, sheet_name).ok_or_else(|| {
        format!("sheet not found: {}", sheet_name)
    })?;

    let sheet_path = match archive.by_name("xl/_rels/workbook.xml.rels") {
        Ok(mut rels_entry) => {
            let mut rels_xml = Vec::new();
            rels_entry
                .read_to_end(&mut rels_xml)
                .map_err(|e| e.to_string())?;
            rels_target(&rels_xml, &r_id).unwrap_or_else(|| {
                format!("worksheets/sheet{}.xml", rid_to_sheet_index(&r_id))
            })
        }
        Err(_) => format!("worksheets/sheet{}.xml", rid_to_sheet_index(&r_id)),
    };

    let mut sst: Vec<String> = Vec::new();
    if let Ok(mut sst_entry) = archive.by_name("xl/sharedStrings.xml") {
        let mut sst_xml = Vec::new();
        sst_entry
            .read_to_end(&mut sst_xml)
            .map_err(|e| e.to_string())?;
        sst = parse_shared_strings(&sst_xml);
    }

    let mut sheet_xml = Vec::new();
    archive
        .by_name(&format!("xl/{}", sheet_path))
        .map_err(|e| e.to_string())?
        .read_to_end(&mut sheet_xml)
        .map_err(|e| e.to_string())?;

    let mut hits = Vec::new();
    if let Some(keyword) = keyword {
        scan_sheet(
            &sheet_xml,
            &sst,
            usize::MAX,
            &mut |_, _| true,
            &mut |row, col, value| {
                if cell_matches(&value, keyword, match_mode) {
                    hits.push(Hit {
                        row: row as i64,
                        col: col as i64,
                        value: value.into_owned(),
                    });
                    hits.len() < max_hits
                } else {
                    true
                }
            },
        )?;
    }

    let (window_row, window_col) = preview_window(hits.first(), start_row, start_col);
    let col_start = window_col.saturating_sub(1);
    let col_end = col_start + preview_cols;

    let mut header_row: Vec<String> = Vec::new();
    let mut data: Vec<Vec<String>> = Vec::new();
    let window_max_row = window_row + preview_rows;
    scan_sheet(
        &sheet_xml,
        &sst,
        window_max_row,
        &mut |row, col| {
            (row == 1 && col <= preview_cols)
                || (row >= window_row
                    && row < window_row + preview_rows
                    && col > col_start
                    && col <= col_end)
        },
        &mut |row, col, value| {
            if row == 1 {
                if header_row.len() < preview_cols {
                    header_row.resize(preview_cols, String::new());
                }
                header_row[col - 1] = value.into_owned();
            } else {
                let row_index = row - window_row;
                if data.len() < row_index + 1 {
                    data.resize(row_index + 1, vec![String::new(); preview_cols]);
                }
                data[row_index][col - col_start - 1] = value.into_owned();
            }
            true
        },
    )?;

    Ok(PreviewResponse {
        hits,
        data: data.into_iter().map(|row| trim_row(&row)).collect(),
        header_row: trim_row(&header_row),
        start_row: window_row,
        start_col: window_col,
    })
}
