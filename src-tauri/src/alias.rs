use encoding_rs::GBK;
use std::fs;

fn ascii_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#')
        || trimmed.starts_with("::")
        || trimmed.to_ascii_uppercase().starts_with("REM ")
}

fn parse_new_format(tokens: &[&str]) -> Vec<(String, String)> {
    let alias = tokens[0];
    if !ascii_token(alias) || tokens.len() < 2 {
        return Vec::new();
    }
    tokens[1..]
        .iter()
        .map(|sheet| (alias.to_string(), sheet.to_string()))
        .collect()
}

fn parse_legacy_format(tokens: &[&str]) -> Vec<(String, String)> {
    let payload = &tokens[2..];
    let mut aliases = Vec::new();
    let mut sheets = Vec::new();
    for token in payload {
        if sheets.is_empty() && ascii_token(token) {
            aliases.push(token.to_string());
        } else {
            sheets.push(token.to_string());
        }
    }
    if aliases.is_empty() || sheets.is_empty() {
        return Vec::new();
    }
    let mut mappings = Vec::new();
    for alias in &aliases {
        for sheet in &sheets {
            mappings.push((alias.clone(), sheet.clone()));
        }
    }
    mappings
}

pub fn parse_alias_text(content: &str) -> Vec<(String, String)> {
    let mut mappings = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || is_comment(line) {
            continue;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 2 {
            continue;
        }
        let line_mappings = if tokens[0].eq_ignore_ascii_case("call") && tokens.len() >= 3 {
            parse_legacy_format(&tokens)
        } else {
            parse_new_format(&tokens)
        };
        mappings.extend(line_mappings);
    }

    let mut seen = std::collections::HashSet::new();
    mappings
        .into_iter()
        .filter(|(alias, sheet)| {
            let key = (alias.to_lowercase(), sheet.clone());
            seen.insert(key)
        })
        .collect()
}

pub fn read_alias_file(path: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    if let Ok(content) = std::str::from_utf8(&bytes) {
        return Ok(content.trim_start_matches('\u{feff}').to_string());
    }
    let (content, _, _) = GBK.decode(&bytes);
    Ok(content.trim_start_matches('\u{feff}').to_string())
}
