use super::{authorized_path, quoted_tokens, read_text_sources, reference_at, scan_references};
use crate::validator_contract::{LocalProof, LocalProofConclusion};
use std::path::Path;

fn lower(criterion: &str) -> String {
    criterion.to_ascii_lowercase()
}

fn has_any(text: &str, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| text.contains(token))
}

fn exact_file_token(criterion: &str) -> Option<String> {
    quoted_tokens(criterion)
        .into_iter()
        .find(|token| Path::new(token).extension().is_some())
}

fn fact_kind(criterion: &str) -> Option<&'static str> {
    let text = lower(criterion);
    let quoted = quoted_tokens(criterion);
    if has_any(&text, &["file exists", "文件存在", "存在文件"])
        && exact_file_token(criterion).is_some()
    {
        return Some("exact_file_exists");
    }
    if quoted.is_empty()
        || !has_any(
            &text,
            &["exists", "defined", "definition", "存在", "定义", "声明"],
        )
    {
        return None;
    }
    if has_any(
        &text,
        &["storage", "localstorage", "sessionstorage", "存储键"],
    ) {
        return Some("exact_storage_key");
    }
    if has_any(
        &text,
        &["identifier", "symbol", "function", "标识符", "符号", "函数"],
    ) {
        return Some("exact_identifier_definition");
    }
    None
}

pub(crate) fn capability(criterion: &str) -> Option<(&'static str, &'static str)> {
    match fact_kind(criterion)? {
        "exact_file_exists" => Some(("exact_file_exists", "authorized exact file existence")),
        "exact_storage_key" => Some(("exact_storage_key", "exact storage API key usage")),
        "exact_identifier_definition" => Some((
            "exact_identifier_definition",
            "exact identifier definition syntax",
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
        "exact_file_exists" => validate_file(root, criterion, authorized_paths),
        "exact_storage_key" => validate_storage(root, criterion, authorized_paths),
        "exact_identifier_definition" => validate_identifier(root, criterion, authorized_paths),
        _ => None,
    }
}

fn validate_file(root: &Path, criterion: &str, authorized_paths: &[String]) -> Option<LocalProof> {
    let relative = exact_file_token(criterion)?;
    let path = authorized_path(root, &relative, authorized_paths)?;
    let exists = path.is_file();
    Some(LocalProof {
        validator: "exact_file_exists",
        conclusion: if exists {
            LocalProofConclusion::Satisfied
        } else {
            LocalProofConclusion::Unsatisfied
        },
        scan_complete: true,
        proof_scope: relative.clone(),
        evidence_references: scan_references(std::slice::from_ref(&relative)),
        expected: format!("文件 {} 存在", relative),
        actual: if exists {
            "精确文件存在".to_string()
        } else {
            "精确文件不存在".to_string()
        },
        suggested_change: format!("在授权范围内提供文件 {}", relative),
    })
}

fn validate_storage(
    root: &Path,
    criterion: &str,
    authorized_paths: &[String],
) -> Option<LocalProof> {
    let key = quoted_tokens(criterion).into_iter().next()?;
    let sources = read_text_sources(root, authorized_paths, None)?;
    let needles = [
        format!("setItem(\"{}\"", key),
        format!("setItem('{}'", key),
        format!("getItem(\"{}\"", key),
        format!("getItem('{}'", key),
        format!("removeItem(\"{}\"", key),
        format!("removeItem('{}'", key),
    ];
    exact_search(
        "exact_storage_key",
        &key,
        &sources,
        authorized_paths,
        &needles,
        "精确存储 API 键存在",
    )
}

fn validate_identifier(
    root: &Path,
    criterion: &str,
    authorized_paths: &[String],
) -> Option<LocalProof> {
    let identifier = quoted_tokens(criterion).into_iter().next()?;
    if !identifier
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
    {
        return None;
    }
    let sources = read_text_sources(root, authorized_paths, None)?;
    let needles = [
        format!("fn {}", identifier),
        format!("function {}", identifier),
        format!("def {}", identifier),
        format!("class {}", identifier),
        format!("struct {}", identifier),
        format!("enum {}", identifier),
        format!("interface {}", identifier),
        format!("type {}", identifier),
        format!("const {}", identifier),
        format!("let {}", identifier),
        format!("var {}", identifier),
    ];
    exact_search(
        "exact_identifier_definition",
        &identifier,
        &sources,
        authorized_paths,
        &needles,
        "精确标识符定义存在",
    )
}

fn exact_search(
    validator: &'static str,
    value: &str,
    sources: &[super::SourceFile],
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
            format!("在 {} 中找到精确定义", source.relative),
        )
    } else {
        (
            LocalProofConclusion::Unsatisfied,
            scan_references(authorized_paths),
            "完整授权范围中未找到精确定义".to_string(),
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
        suggested_change: format!("在授权范围内提供 {} 的明确定义", value),
    })
}
