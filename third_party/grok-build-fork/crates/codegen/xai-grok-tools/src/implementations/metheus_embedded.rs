//! Session-scoped filesystem policy for the Metheus embedded runtime.
//!
//! The normal Grok tools keep their upstream behavior. When this resource is
//! present, `list_dir` and `grep` use the bounded in-process implementations
//! below instead of touching arbitrary host paths or spawning `rg`.

use crate::implementations::grok_build::grep::{GrepSearchInput, OutputMode};
use crate::types::output::{GrepFileMatch, GrepLineMatch, GrepSearchOutput};
use globset::{Glob, GlobMatcher};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SEARCHED_FILES: usize = 10_000;
const MAX_LIST_CHARS: usize = 10_000;
const MAX_SEARCH_BYTES: usize = 40_000;
const MAX_LIST_DEPTH: usize = 8;
pub const EMBEDDED_PATH_NOT_FOUND: &str = "requested project path does not exist";

#[derive(Clone, Debug)]
pub struct EmbeddedFilePolicy(Arc<EmbeddedFilePolicyInner>);

#[derive(Debug)]
struct EmbeddedFilePolicyInner {
    root: PathBuf,
    authorized_writes: BTreeSet<PathBuf>,
}

impl EmbeddedFilePolicy {
    pub fn new(root: &Path, authorized: &[PathBuf]) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve embedded project root: {error}"))?;
        if !root.is_dir() {
            return Err("embedded project root is not a directory".to_string());
        }
        let mut authorized_writes = BTreeSet::new();
        for path in authorized {
            authorized_writes.insert(resolve_write_target(&root, path)?);
        }
        Ok(Self(Arc::new(EmbeddedFilePolicyInner {
            root,
            authorized_writes,
        })))
    }

    pub fn root(&self) -> &Path {
        &self.0.root
    }

    pub fn read_text_file(
        &self,
        path: &Path,
        line: Option<u32>,
        limit: Option<u32>,
    ) -> Result<String, String> {
        let path = self.resolve_read_target(path)?;
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("cannot inspect requested project file: {error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err("only bounded regular project files may be read".to_string());
        }
        if limit == Some(0) {
            return Ok(String::new());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|_| "file is not readable UTF-8 text".to_string())?;
        if line.is_none() && limit.is_none() {
            return Ok(content);
        }
        let start = line.unwrap_or(1).saturating_sub(1) as usize;
        Ok(content
            .lines()
            .skip(start)
            .take(limit.unwrap_or(u32::MAX) as usize)
            .collect::<Vec<_>>()
            .join("\n"))
    }

    pub fn write_text_file(&self, path: &Path, content: &str) -> Result<PathBuf, String> {
        if content.len() as u64 > MAX_FILE_BYTES {
            return Err("write exceeds the embedded file size limit".to_string());
        }
        let target = resolve_write_target(&self.0.root, path)?;
        if !self.0.authorized_writes.contains(&target) {
            return Err("write path is not in the frozen authorization set".to_string());
        }
        std::fs::write(&target, content)
            .map_err(|error| format!("authorized file write failed: {error}"))?;
        Ok(target)
    }

    pub fn list_dir(&self, target: &str) -> Result<(PathBuf, String), String> {
        let target = self.resolve_read_target(Path::new(target))?;
        if !target.is_dir() {
            return Err("requested project path is not a directory".to_string());
        }
        let mut entries = Vec::new();
        let walker = WalkBuilder::new(&target)
            .follow_links(false)
            .max_depth(Some(MAX_LIST_DEPTH))
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .parents(false)
            .build();
        for entry in walker.filter_map(Result::ok).skip(1) {
            let path = entry.path();
            if !path.starts_with(&target) {
                continue;
            }
            let relative = path.strip_prefix(&target).unwrap_or(path);
            let suffix = entry
                .file_type()
                .map(|kind| {
                    if kind.is_dir() {
                        "/"
                    } else if kind.is_symlink() {
                        "@"
                    } else {
                        ""
                    }
                })
                .unwrap_or("?");
            entries.push(format!("{}{suffix}", relative.to_string_lossy()));
        }
        entries.sort();
        let mut output = String::new();
        for entry in entries {
            let next_len = output.len().saturating_add(entry.len()).saturating_add(1);
            if next_len > MAX_LIST_CHARS {
                output.push_str("...[truncated]\n");
                break;
            }
            output.push_str(&entry);
            output.push('\n');
        }
        if output.is_empty() {
            output.push_str("Directory is empty.");
        }
        Ok((target, output.trim_end().to_string()))
    }

    pub fn grep(&self, input: &GrepSearchInput) -> GrepSearchOutput {
        match self.grep_inner(input) {
            Ok(output) => output,
            Err(message) => GrepSearchOutput {
                stdout: format!("Error calling tool: {message}").into_bytes(),
                stderr: message.into_bytes(),
                exit_code: 2,
                match_count: 0,
                file_matches: Vec::new(),
            },
        }
    }

    fn grep_inner(&self, input: &GrepSearchInput) -> Result<GrepSearchOutput, String> {
        if input.pattern.trim().is_empty() {
            return Err("search pattern must not be empty".to_string());
        }
        let regex = RegexBuilder::new(&input.pattern)
            .case_insensitive(input.case_insensitive.unwrap_or(false))
            .dot_matches_new_line(input.multiline.unwrap_or(false))
            .build()
            .map_err(|error| format!("invalid regular expression: {error}"))?;
        let target = self.resolve_read_target(Path::new(input.path.as_deref().unwrap_or(".")))?;
        let glob = input
            .glob
            .as_deref()
            .map(Glob::new)
            .transpose()
            .map_err(|error| format!("invalid glob: {error}"))?
            .map(|glob| glob.compile_matcher());
        let extensions = input.r#type.as_deref().map(type_extensions).transpose()?;
        let mode = input.output_mode.clone().unwrap_or_default();
        let default_limit = if mode == OutputMode::Content {
            200
        } else {
            500
        };
        let hard_limit = if mode == OutputMode::Content {
            2_000
        } else {
            10_000
        };
        let line_limit = input.head_limit.unwrap_or(default_limit).min(hard_limit);
        let mut files = Vec::new();
        if target.is_file() {
            files.push(target.clone());
        } else if target.is_dir() {
            let walker = WalkBuilder::new(&target)
                .follow_links(false)
                .hidden(false)
                .git_ignore(true)
                .git_global(false)
                .parents(false)
                .build();
            for entry in walker.filter_map(Result::ok) {
                if files.len() >= MAX_SEARCHED_FILES {
                    break;
                }
                if entry.file_type().is_some_and(|kind| kind.is_file()) {
                    files.push(entry.into_path());
                }
            }
        } else {
            return Err("search target is neither a file nor a directory".to_string());
        }
        files.sort();

        let mut by_file: BTreeMap<String, Vec<GrepLineMatch>> = BTreeMap::new();
        for path in files.into_iter().take(MAX_SEARCHED_FILES) {
            if !self.path_is_searchable(&path, glob.as_ref(), extensions.as_deref()) {
                continue;
            }
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let relative = path
                .strip_prefix(&self.0.root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let matches = collect_matches(&regex, &content, input.multiline.unwrap_or(false));
            if !matches.is_empty() {
                by_file.insert(relative, matches);
            }
        }

        let mut body = Vec::new();
        let mut file_matches = Vec::new();
        let mut match_count = 0usize;
        for (path, matches) in by_file {
            match_count = match_count.saturating_add(matches.len());
            match mode {
                OutputMode::Content => {
                    for item in &matches {
                        if body.len() >= line_limit {
                            break;
                        }
                        body.push(format!("{path}:{}:{}", item.line_number, item.content));
                    }
                }
                OutputMode::FilesWithMatches => {
                    if body.len() < line_limit {
                        body.push(path.clone());
                    }
                }
                OutputMode::Count => {
                    if body.len() < line_limit {
                        body.push(format!("{path}:{}", matches.len()));
                    }
                }
            }
            file_matches.push(GrepFileMatch { path, matches });
            if body.len() >= line_limit {
                break;
            }
        }
        let mut formatted = if body.is_empty() {
            "No matches found".to_string()
        } else {
            body.join("\n")
        };
        if formatted.len() > MAX_SEARCH_BYTES {
            formatted.truncate(floor_char_boundary(&formatted, MAX_SEARCH_BYTES));
            formatted.push_str("\n...[truncated]");
        }
        Ok(GrepSearchOutput {
            stdout: format!(
                "<workspace_result workspace_path=\"{}\">\n{}\n</workspace_result>",
                self.0.root.display(),
                formatted
            )
            .into_bytes(),
            stderr: Vec::new(),
            exit_code: if match_count == 0 { 1 } else { 0 },
            match_count,
            file_matches,
        })
    }

    fn path_is_searchable(
        &self,
        path: &Path,
        glob: Option<&GlobMatcher>,
        extensions: Option<&[&str]>,
    ) -> bool {
        let relative = path.strip_prefix(&self.0.root).unwrap_or(path);
        if glob.is_some_and(|matcher| !matcher.is_match(relative)) {
            return false;
        }
        extensions.is_none_or(|allowed| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| allowed.contains(&extension))
        })
    }

    fn resolve_read_target(&self, path: &Path) -> Result<PathBuf, String> {
        reject_parent_components(path)?;
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.0.root.join(path)
        };
        let resolved = candidate.canonicalize().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                EMBEDDED_PATH_NOT_FOUND.to_string()
            } else {
                format!("cannot resolve requested project path: {error}")
            }
        })?;
        if !resolved.starts_with(&self.0.root) {
            return Err("path is outside the project root".to_string());
        }
        Ok(resolved)
    }
}

fn collect_matches(regex: &Regex, content: &str, multiline: bool) -> Vec<GrepLineMatch> {
    if multiline {
        return regex
            .find_iter(content)
            .map(|found| GrepLineMatch {
                line_number: content[..found.start()].lines().count().saturating_add(1),
                content: found.as_str().replace('\n', "\\n"),
            })
            .collect();
    }
    content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            regex.is_match(line).then(|| GrepLineMatch {
                line_number: index + 1,
                content: line.to_string(),
            })
        })
        .collect()
}

fn type_extensions(name: &str) -> Result<Vec<&'static str>, String> {
    let extensions = match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => vec!["rs"],
        "python" | "py" => vec!["py", "pyi"],
        "javascript" | "js" => vec!["js", "jsx", "mjs", "cjs"],
        "typescript" | "ts" => vec!["ts", "tsx", "mts", "cts"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        "c" => vec!["c", "h"],
        "cpp" | "c++" => vec!["cc", "cpp", "cxx", "hh", "hpp", "hxx"],
        "json" => vec!["json"],
        "yaml" | "yml" => vec!["yaml", "yml"],
        "toml" => vec!["toml"],
        "markdown" | "md" => vec!["md", "mdx"],
        other => return Err(format!("unsupported embedded file type filter: {other}")),
    };
    Ok(extensions)
}

fn resolve_write_target(root: &Path, path: &Path) -> Result<PathBuf, String> {
    reject_parent_components(path)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if let Ok(metadata) = std::fs::symlink_metadata(&candidate) {
        if metadata.file_type().is_symlink() {
            return Err("symbolic-link write targets are not allowed".to_string());
        }
        let resolved = candidate
            .canonicalize()
            .map_err(|error| format!("cannot resolve write target: {error}"))?;
        if !resolved.starts_with(root) || !resolved.is_file() {
            return Err("write target must be a regular project file".to_string());
        }
        return Ok(resolved);
    }
    let file_name = candidate
        .file_name()
        .ok_or_else(|| "write target must be a file".to_string())?;
    let parent = candidate
        .parent()
        .ok_or_else(|| "write target parent is missing".to_string())?
        .canonicalize()
        .map_err(|_| "write target parent must already exist".to_string())?;
    if !parent.starts_with(root) {
        return Err("write target is outside the project root".to_string());
    }
    Ok(parent.join(file_name))
}

fn reject_parent_components(path: &Path) -> Result<(), String> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("parent path components are not allowed".to_string());
    }
    Ok(())
}

fn floor_char_boundary(value: &str, maximum: usize) -> usize {
    let mut end = maximum.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_lists_and_searches_without_leaving_root() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        std::fs::create_dir(root.path().join("src"))?;
        std::fs::write(root.path().join("src/lib.rs"), "fn embedded_probe() {}\n")?;
        let policy = EmbeddedFilePolicy::new(root.path(), &[])?;
        let (_, listing) = policy.list_dir(".")?;
        assert!(listing.contains("src/lib.rs"));
        let output = policy.grep(&GrepSearchInput {
            pattern: "embedded_probe".to_string(),
            path: None,
            glob: Some("*.rs".to_string()),
            output_mode: None,
            before_context: None,
            after_context: None,
            context: None,
            case_insensitive: None,
            r#type: None,
            head_limit: None,
            multiline: None,
        });
        assert_eq!(output.exit_code, 0);
        assert!(String::from_utf8(output.stdout)?.contains("src/lib.rs:1"));
        assert!(policy.list_dir("../").is_err());
        Ok(())
    }

    #[test]
    fn policy_rejects_unauthorized_and_symlink_writes() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::fs::write(outside.path().join("outside.txt"), "outside")?;
        let policy = EmbeddedFilePolicy::new(root.path(), &[PathBuf::from("allowed.txt")])?;
        policy.write_text_file(Path::new("allowed.txt"), "ok")?;
        assert!(
            policy
                .write_text_file(Path::new("other.txt"), "no")
                .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("outside.txt"),
                root.path().join("linked.txt"),
            )?;
            assert!(
                policy
                    .write_text_file(Path::new("linked.txt"), "no")
                    .is_err()
            );
            assert!(
                policy
                    .read_text_file(Path::new("linked.txt"), None, None)
                    .is_err()
            );
            std::fs::remove_file(root.path().join("allowed.txt"))?;
            std::os::unix::fs::symlink(
                outside.path().join("outside.txt"),
                root.path().join("allowed.txt"),
            )?;
            assert!(
                policy
                    .write_text_file(Path::new("allowed.txt"), "no")
                    .is_err()
            );
        }
        assert!(
            policy
                .read_text_file(&outside.path().join("outside.txt"), None, None)
                .is_err()
        );
        assert!(
            EmbeddedFilePolicy::new(root.path(), &[outside.path().join("outside.txt")]).is_err()
        );
        Ok(())
    }
}
