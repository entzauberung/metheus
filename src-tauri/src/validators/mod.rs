use crate::project::{EvidenceSourceKind, ReviewEvidenceReference};
use std::path::{Component, Path, PathBuf};

pub mod exact_fact;
pub mod web_static;

#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    pub relative: String,
    pub content: String,
}

pub(crate) fn quoted_tokens(criterion: &str) -> Vec<String> {
    criterion
        .split('`')
        .enumerate()
        .filter_map(|(index, value)| {
            let value = value.trim();
            (index % 2 == 1 && !value.is_empty()).then(|| value.to_string())
        })
        .collect()
}

pub(crate) fn authorized_path(
    root: &Path,
    relative: &str,
    authorized_paths: &[String],
) -> Option<PathBuf> {
    if !authorized_paths.iter().any(|path| path == relative) {
        return None;
    }
    let parsed = Path::new(relative);
    if parsed.is_absolute()
        || parsed.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }
    Some(root.join(parsed))
}

pub(crate) fn read_text_sources(
    root: &Path,
    authorized_paths: &[String],
    extensions: Option<&[&str]>,
) -> Option<Vec<SourceFile>> {
    let mut sources = Vec::new();
    for relative in authorized_paths {
        let path = authorized_path(root, relative, authorized_paths)?;
        if let Some(extensions) = extensions {
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !extensions.iter().any(|candidate| *candidate == extension) {
                continue;
            }
        }
        if !path.is_file() {
            continue;
        }
        sources.push(SourceFile {
            relative: relative.clone(),
            content: std::fs::read_to_string(path).ok()?,
        });
    }
    Some(sources)
}

pub(crate) fn reference_at(file: &SourceFile, byte_offset: usize) -> ReviewEvidenceReference {
    let line = file.content[..byte_offset.min(file.content.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count() as u32
        + 1;
    ReviewEvidenceReference {
        block_id: format!("local:{}:{}", file.relative, line),
        source_kind: EvidenceSourceKind::CurrentFileSnippet,
        file: file.relative.clone(),
        start_line: Some(line),
        end_line: Some(line),
    }
}

pub(crate) fn scan_references(authorized_paths: &[String]) -> Vec<ReviewEvidenceReference> {
    authorized_paths
        .iter()
        .map(|file| ReviewEvidenceReference {
            block_id: format!("local-scan:{}", file),
            source_kind: EvidenceSourceKind::CurrentFileSnippet,
            file: file.clone(),
            start_line: None,
            end_line: None,
        })
        .collect()
}

pub(crate) fn capability(criterion: &str) -> Option<(&'static str, &'static str)> {
    exact_fact::capability(criterion).or_else(|| web_static::capability(criterion))
}

pub(crate) fn validate(
    root: &Path,
    criterion: &str,
    authorized_paths: &[String],
) -> Option<crate::validator_contract::LocalProof> {
    exact_fact::validate(root, criterion, authorized_paths)
        .or_else(|| web_static::validate(root, criterion, authorized_paths))
}
