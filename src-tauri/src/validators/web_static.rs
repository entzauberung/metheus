use super::{quoted_tokens, read_text_sources, reference_at, scan_references, SourceFile};
use crate::validator_contract::{LocalProof, LocalProofConclusion};
use std::path::Path;

fn has_any(text: &str, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| text.contains(token))
}

fn fact_kind(criterion: &str) -> Option<&'static str> {
    let text = criterion.to_ascii_lowercase();
    let quoted = quoted_tokens(criterion);
    if !quoted.is_empty()
        && has_any(&text, &["dom", "节点"])
        && has_any(&text, &["id", "标识", "存在", "exists", "contains", "包含"])
    {
        return Some("exact_dom_id");
    }
    if quoted.iter().any(|token| token.starts_with("--"))
        && has_any(
            &text,
            &["css variable", "css 变量", "css变量", "自定义属性"],
        )
    {
        return Some("css_variable_definition");
    }
    if has_any(
        &text,
        &["hardcoded color", "hard-coded color", "硬编码颜色"],
    ) && has_any(
        &text,
        &[
            "no ",
            "must not",
            "without",
            "不得",
            "禁止",
            "不存在",
            "没有",
        ],
    ) {
        return Some("css_hardcoded_color_absence");
    }
    None
}

pub(crate) fn capability(criterion: &str) -> Option<(&'static str, &'static str)> {
    match fact_kind(criterion)? {
        "exact_dom_id" => Some(("exact_dom_id", "exact HTML id attribute")),
        "css_variable_definition" => Some((
            "css_variable_definition",
            "exact CSS custom property definition",
        )),
        "css_hardcoded_color_absence" => Some((
            "css_hardcoded_color_absence",
            "complete CSS declaration scan",
        )),
        _ => None,
    }
}

pub(crate) fn validate(
    root: &Path,
    criterion: &str,
    authorized_paths: &[String],
) -> Option<LocalProof> {
    match fact_kind(criterion)? {
        "exact_dom_id" => exact_dom_id(root, criterion, authorized_paths),
        "css_variable_definition" => css_variable(root, criterion, authorized_paths),
        "css_hardcoded_color_absence" => hardcoded_colors(root, authorized_paths),
        _ => None,
    }
}

fn exact_dom_id(root: &Path, criterion: &str, authorized_paths: &[String]) -> Option<LocalProof> {
    let id = quoted_tokens(criterion).into_iter().next()?;
    let sources = read_text_sources(root, authorized_paths, Some(&["html", "htm", "tsx", "jsx"]))?;
    let needles = [format!("id=\"{}\"", id), format!("id='{}'", id)];
    exact_web_search(
        "exact_dom_id",
        &id,
        &sources,
        authorized_paths,
        &needles,
        "精确 DOM id 属性",
    )
}

fn css_variable(root: &Path, criterion: &str, authorized_paths: &[String]) -> Option<LocalProof> {
    let variable = quoted_tokens(criterion)
        .into_iter()
        .find(|token| token.starts_with("--"))?;
    let sources = read_text_sources(
        root,
        authorized_paths,
        Some(&["css", "scss", "sass", "less", "html", "htm"]),
    )?;
    let needles = [format!("{}:", variable), format!("{} :", variable)];
    exact_web_search(
        "css_variable_definition",
        &variable,
        &sources,
        authorized_paths,
        &needles,
        "CSS 自定义属性定义",
    )
}

fn exact_web_search(
    validator: &'static str,
    value: &str,
    sources: &[SourceFile],
    authorized_paths: &[String],
    needles: &[String],
    expected: &str,
) -> Option<LocalProof> {
    let found = sources.iter().find_map(|source| {
        needles
            .iter()
            .find_map(|needle| source.content.find(needle).map(|offset| (source, offset)))
    });
    let (conclusion, evidence_references, actual) = if let Some((source, offset)) = found {
        (
            LocalProofConclusion::Satisfied,
            vec![reference_at(source, offset)],
            format!("在 {} 找到精确事实", source.relative),
        )
    } else {
        (
            LocalProofConclusion::Unsatisfied,
            scan_references(authorized_paths),
            "完整目标范围中未找到精确事实".to_string(),
        )
    };
    Some(LocalProof {
        validator,
        conclusion,
        scan_complete: true,
        proof_scope: authorized_paths.join(","),
        evidence_references,
        expected: format!("{}：{}", expected, value),
        actual,
        suggested_change: format!("在授权范围内补齐 {}", value),
    })
}

fn hardcoded_colors(root: &Path, authorized_paths: &[String]) -> Option<LocalProof> {
    if authorized_paths.iter().any(|path| {
        !matches!(
            Path::new(path).extension().and_then(|value| value.to_str()),
            Some("css" | "html" | "htm")
        )
    }) {
        return None;
    }
    let sources = read_text_sources(root, authorized_paths, None)?;
    let violation = sources.iter().find_map(find_hardcoded_color);
    let (conclusion, references, actual) = if let Some((source, offset, token)) = violation {
        (
            LocalProofConclusion::Unsatisfied,
            vec![reference_at(source, offset)],
            format!("发现硬编码颜色 {}", token),
        )
    } else {
        (
            LocalProofConclusion::Satisfied,
            scan_references(authorized_paths),
            "完整 CSS 声明范围未发现硬编码颜色".to_string(),
        )
    };
    Some(LocalProof {
        validator: "css_hardcoded_color_absence",
        conclusion,
        scan_complete: true,
        proof_scope: authorized_paths.join(","),
        evidence_references: references,
        expected: "非变量 CSS 声明不包含十六进制、rgb 或 hsl 硬编码颜色".to_string(),
        actual,
        suggested_change: "将样式声明改为引用 CSS 变量".to_string(),
    })
}

fn find_hardcoded_color(source: &SourceFile) -> Option<(&SourceFile, usize, String)> {
    let extension = Path::new(&source.relative)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if matches!(extension, "html" | "htm") {
        return find_html_color(source);
    }
    find_css_color(source, 0, source.content.len(), true)
}

fn find_html_color(source: &SourceFile) -> Option<(&SourceFile, usize, String)> {
    let lower = source.content.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative_open) = lower[cursor..].find("<style") {
        let tag_start = cursor + relative_open;
        let content_start = lower[tag_start..].find('>')? + tag_start + 1;
        let content_end = lower[content_start..].find("</style>")? + content_start;
        if let Some(found) = find_css_color(source, content_start, content_end, true) {
            return Some(found);
        }
        cursor = content_end + "</style>".len();
    }

    cursor = 0;
    while let Some(relative_style) = lower[cursor..].find("style=") {
        let equals = cursor + relative_style + "style=".len();
        let quote_at = lower[equals..]
            .char_indices()
            .find(|(_, ch)| !ch.is_ascii_whitespace())
            .map(|(offset, _)| equals + offset)?;
        let quote = source.content.as_bytes()[quote_at];
        if !matches!(quote, b'\'' | b'\"') {
            cursor = quote_at.saturating_add(1);
            continue;
        }
        let value_start = quote_at + 1;
        let value_end = source.content[value_start..].find(quote as char)? + value_start;
        if let Some(found) = find_css_color(source, value_start, value_end, false) {
            return Some(found);
        }
        cursor = value_end + 1;
    }
    None
}

fn find_css_color(
    source: &SourceFile,
    start: usize,
    end: usize,
    require_declaration_block: bool,
) -> Option<(&SourceFile, usize, String)> {
    let bytes = source.content.as_bytes();
    let mut index = start;
    let mut in_comment = false;
    let mut quote = None;
    let mut brace_depth = 0_u32;
    while index < end {
        if in_comment {
            if index + 1 < bytes.len() && bytes[index] == b'*' && bytes[index + 1] == b'/' {
                in_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if quote.is_none()
            && index + 1 < bytes.len()
            && bytes[index] == b'/'
            && bytes[index + 1] == b'*'
        {
            in_comment = true;
            index += 2;
            continue;
        }
        if matches!(bytes[index], b'\'' | b'\"') {
            if quote == Some(bytes[index]) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(bytes[index]);
            }
            index += 1;
            continue;
        }
        if quote.is_some() {
            index += 1;
            continue;
        }
        if bytes[index] == b'{' {
            brace_depth = brace_depth.saturating_add(1);
            index += 1;
            continue;
        }
        if bytes[index] == b'}' {
            brace_depth = brace_depth.saturating_sub(1);
            index += 1;
            continue;
        }
        if bytes[index] == b':' && (!require_declaration_block || brace_depth > 0) {
            let property_start = source.content[start..index]
                .rfind([';', '{'])
                .map(|value| start + value + 1)
                .unwrap_or(start);
            let property = source.content[property_start..index].trim();
            let value_end = source.content[index + 1..end]
                .find([';', '}'])
                .map(|value| index + 1 + value)
                .unwrap_or(end);
            if !property.starts_with("--") {
                let value = &source.content[index + 1..value_end];
                if let Some((relative, token)) = color_token(value) {
                    return Some((source, index + 1 + relative, token));
                }
            }
            index = value_end;
            continue;
        }
        index += 1;
    }
    None
}

fn color_token(value: &str) -> Option<(usize, String)> {
    let lower = value.to_ascii_lowercase();
    for function in ["rgb(", "rgba(", "hsl(", "hsla("] {
        if let Some(offset) = lower.find(function) {
            return Some((offset, function.trim_end_matches('(').to_string()));
        }
    }
    for (offset, _) in value.match_indices('#') {
        let digits = value[offset + 1..]
            .chars()
            .take_while(|ch| ch.is_ascii_hexdigit())
            .count();
        if matches!(digits, 3 | 4 | 6 | 8) {
            return Some((offset, value[offset..offset + digits + 1].to_string()));
        }
    }
    None
}
