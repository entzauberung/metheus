use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_RESPONSE_BYTES: usize = 800 * 1024;
const CAPABILITY_TEST_PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0,
    0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 8, 215, 99, 248, 207, 192, 0, 0, 3, 1, 1,
    0, 24, 221, 141, 24, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

#[derive(Debug, Clone)]
struct LoadedVisualEvidence {
    reference: crate::project::VisualEvidenceReference,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct VisionCriterionResult {
    pub criterion_index: u32,
    pub status: crate::project::VisualReviewStatus,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct VisionReviewResult {
    pub model: String,
    pub latency_ms: u64,
    pub criteria: Vec<VisionCriterionResult>,
    pub evidence: Vec<crate::project::VisualEvidenceReference>,
}

#[derive(Debug, Deserialize)]
struct VisionProtocolResponse {
    #[serde(default)]
    image_detected: bool,
    #[serde(default)]
    criteria: Vec<VisionCriterionResult>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    id: String,
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: OpenAiUsage,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}

#[derive(Debug, Default, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    total_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
struct VisionCallError {
    kind: crate::settings::ModelConnectionErrorKind,
    message: String,
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn criteria_for_indexes(
    task: &crate::project::Subtask,
    criterion_indexes: &[u32],
) -> Result<Vec<(u32, String)>, String> {
    let mut seen = BTreeSet::new();
    criterion_indexes
        .iter()
        .map(|index| {
            if *index == 0 {
                return Err("视觉审查验收项编号必须从 1 开始".to_string());
            }
            let criterion = task
                .acceptance_criteria
                .get(index.saturating_sub(1) as usize)
                .cloned()
                .ok_or_else(|| format!("视觉审查验收项编号越界：{index}"))?;
            if !seen.insert(*index) {
                return Err(format!("视觉审查验收项编号重复：{index}"));
            }
            Ok((*index, criterion))
        })
        .collect()
}

fn canonical_project_root(project_root: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(project_root)
        .map_err(|error| format!("无法解析视觉证据项目根目录：{error}"))?;
    if !root.is_dir() {
        return Err("视觉证据项目根目录不是目录".to_string());
    }
    Ok(root)
}

fn collect_visual_evidence_loaded(
    project_root: &str,
    task: &crate::project::Subtask,
    settings: &crate::settings::VisionModelSettings,
) -> Result<Vec<LoadedVisualEvidence>, String> {
    let root = canonical_project_root(project_root)?;
    let mut loaded = Vec::new();
    let mut canonical_seen = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for declared in crate::review_evidence::declared_visual_evidence_paths(task) {
        if loaded.len() >= settings.max_images as usize {
            return Err(format!(
                "声明的视觉证据超过数量限制 {}",
                settings.max_images
            ));
        }
        let relative = Path::new(&declared);
        if relative.is_absolute() {
            return Err(format!("视觉证据必须是项目内相对路径：{declared}"));
        }
        let joined = root.join(relative);
        let canonical = std::fs::canonicalize(&joined)
            .map_err(|error| format!("无法读取声明的视觉证据 {declared}：{error}"))?;
        if !canonical.starts_with(&root) {
            return Err(format!("声明的视觉证据越过项目根目录：{declared}"));
        }
        if !canonical_seen.insert(canonical.clone()) {
            continue;
        }
        let metadata = std::fs::metadata(&canonical)
            .map_err(|error| format!("无法读取视觉证据元数据 {declared}：{error}"))?;
        if !metadata.is_file() {
            return Err(format!("声明的视觉证据不是文件：{declared}"));
        }
        if metadata.len() > settings.max_image_bytes {
            return Err(format!("视觉证据超过单文件大小限制：{declared}"));
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > settings.max_total_bytes {
            return Err("视觉证据超过总大小限制".to_string());
        }
        let bytes = std::fs::read(&canonical)
            .map_err(|error| format!("读取视觉证据失败 {declared}：{error}"))?;
        let mime = image_mime(&bytes)
            .ok_or_else(|| format!("视觉证据的文件内容不是受支持图片：{declared}"))?;
        let extension = relative
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let extension_matches = match mime {
            "image/png" => extension == "png",
            "image/jpeg" => matches!(extension.as_str(), "jpg" | "jpeg"),
            "image/webp" => extension == "webp",
            _ => false,
        };
        if !extension_matches {
            return Err(format!("视觉证据扩展名与 MIME 不一致：{declared}"));
        }
        loaded.push(LoadedVisualEvidence {
            reference: crate::project::VisualEvidenceReference {
                path: declared,
                sha256: format!("sha256:{:x}", Sha256::digest(&bytes)),
                mime: mime.to_string(),
                size_bytes: bytes.len() as u64,
            },
            bytes,
        });
    }
    Ok(loaded)
}

pub(crate) fn collect_visual_evidence(
    project_root: &str,
    task: &crate::project::Subtask,
    settings: &crate::settings::VisionModelSettings,
) -> Result<Vec<crate::project::VisualEvidenceReference>, String> {
    collect_visual_evidence_loaded(project_root, task, settings)
        .map(|items| items.into_iter().map(|item| item.reference).collect())
}

pub(crate) fn validate_visual_evidence_fingerprints(
    project_root: &str,
    task: &crate::project::Subtask,
    evidence: &[crate::project::VisualEvidenceReference],
) -> Result<(), String> {
    if evidence.is_empty() {
        return Err("视觉结果缺少声明文件 fingerprint".to_string());
    }
    let root = canonical_project_root(project_root)?;
    let declared = crate::review_evidence::declared_visual_evidence_paths(task)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for reference in evidence {
        if !declared.contains(&reference.path) {
            return Err(format!(
                "视觉结果引用了任务未声明的图片：{}",
                reference.path
            ));
        }
        if !seen.insert(reference.path.clone()) {
            return Err(format!("视觉结果包含重复图片引用：{}", reference.path));
        }
        let relative = Path::new(&reference.path);
        if relative.is_absolute() {
            return Err(format!(
                "视觉结果图片必须是项目内相对路径：{}",
                reference.path
            ));
        }
        let canonical = std::fs::canonicalize(root.join(relative))
            .map_err(|error| format!("无法复核视觉证据 {}：{error}", reference.path))?;
        if !canonical.starts_with(&root) {
            return Err(format!("视觉结果图片越过项目根目录：{}", reference.path));
        }
        if !canonical.is_file() {
            return Err(format!("视觉结果图片不是文件：{}", reference.path));
        }
        let bytes = std::fs::read(&canonical)
            .map_err(|error| format!("无法复核视觉证据 {}：{error}", reference.path))?;
        let mime = image_mime(&bytes)
            .ok_or_else(|| format!("视觉结果图片内容已不再是受支持图片：{}", reference.path))?;
        if mime != reference.mime || bytes.len() as u64 != reference.size_bytes {
            return Err(format!(
                "视觉证据 MIME 或大小 fingerprint 已变化：{}",
                reference.path
            ));
        }
        let sha256 = format!("sha256:{:x}", Sha256::digest(&bytes));
        if sha256 != reference.sha256 {
            return Err(format!(
                "视觉证据 sha256 fingerprint 已变化：{}",
                reference.path
            ));
        }
    }
    Ok(())
}

pub(crate) fn reconcile_visual_status(
    ai_status: &crate::project::AcceptanceStatus,
    visual_status: crate::project::VisualReviewStatus,
) -> crate::project::VisualReviewStatus {
    if matches!(
        (ai_status, &visual_status),
        (
            crate::project::AcceptanceStatus::AiProvisionallySatisfied,
            crate::project::VisualReviewStatus::Unsatisfied
        ) | (
            crate::project::AcceptanceStatus::Unsatisfied
                | crate::project::AcceptanceStatus::Contradictory,
            crate::project::VisualReviewStatus::Satisfied
        )
    ) {
        crate::project::VisualReviewStatus::Conflict
    } else {
        visual_status
    }
}

fn metadata(
    call_id: String,
    context: crate::cost_ledger::ModelCallContext,
    model: String,
    response_id: String,
    started_at: String,
    elapsed_ms: u64,
    usage: Option<crate::cost_ledger::ProviderUsage>,
    failure_kind: String,
) -> crate::cost_ledger::ModelCallMetadata {
    crate::cost_ledger::ModelCallMetadata {
        call_id,
        context,
        model,
        provider_response_id: response_id,
        started_at,
        ended_at: chrono::Utc::now().to_rfc3339(),
        elapsed_ms,
        usage,
        failure_kind,
    }
}

#[allow(clippy::too_many_arguments)]
fn record_vision_failure(
    call_id: &str,
    context: &crate::cost_ledger::ModelCallContext,
    model: &str,
    response_id: String,
    started_at: &str,
    started: Instant,
    usage: Option<crate::cost_ledger::ProviderUsage>,
    kind: crate::settings::ModelConnectionErrorKind,
    message: impl Into<String>,
) -> VisionCallError {
    let call = metadata(
        call_id.to_string(),
        context.clone(),
        model.to_string(),
        response_id,
        started_at.to_string(),
        started.elapsed().as_millis() as u64,
        usage,
        format!("{kind:?}"),
    );
    crate::cost_ledger::record_metadata_best_effort(&call);
    VisionCallError {
        kind,
        message: message.into(),
    }
}

async fn call_vision_model(
    settings: &crate::settings::VisionModelSettings,
    api_key: &str,
    images: &[LoadedVisualEvidence],
    criteria: &[(u32, String)],
    capability_test: bool,
    context: crate::cost_ledger::ModelCallContext,
) -> Result<
    (
        VisionProtocolResponse,
        crate::cost_ledger::ModelCallMetadata,
    ),
    VisionCallError,
> {
    let call_id = format!("vision-{}", uuid::Uuid::new_v4());
    let started_at = chrono::Utc::now().to_rfc3339();
    let started = Instant::now();
    let prompt = if capability_test {
        "Inspect the attached image. Return only JSON: {\"image_detected\":true,\"criteria\":[]}. Do not claim success unless image pixels were available."
            .to_string()
    } else {
        format!(
            "Inspect only the attached declared screenshots. Return only JSON with image_detected=true and criteria entries shaped as {{criterion_index,status,summary}}. status must be Satisfied, Unsatisfied, EvidenceInsufficient, or Unavailable. Criteria: {}",
            serde_json::to_string(criteria).unwrap_or_default()
        )
    };
    let mut content = vec![serde_json::json!({"type":"text","text":prompt})];
    for image in images {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
        content.push(serde_json::json!({
            "type": "image_url",
            "image_url": {"url": format!("data:{};base64,{}", image.reference.mime, encoded)}
        }));
    }
    let body = serde_json::json!({
        "model": settings.model,
        "temperature": 0,
        "response_format": {"type":"json_object"},
        "messages": [{"role":"user","content":content}]
    });
    let request = reqwest::Client::new()
        .post(&settings.request_url)
        .bearer_auth(api_key)
        .timeout(Duration::from_secs(settings.timeout_secs))
        .json(&body)
        .send()
        .await;
    let response = match request {
        Ok(response) => response,
        Err(error) => {
            let kind = if error.is_timeout() {
                crate::settings::ModelConnectionErrorKind::Timeout
            } else {
                crate::settings::ModelConnectionErrorKind::Network
            };
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                String::new(),
                &started_at,
                started,
                None,
                kind.clone(),
                format!("视觉模型请求失败（{kind:?}）"),
            ));
        }
    };
    if !response.status().is_success() {
        let status = response.status();
        let kind = match status.as_u16() {
            401 | 403 => crate::settings::ModelConnectionErrorKind::Authentication,
            402 => crate::settings::ModelConnectionErrorKind::QuotaExceeded,
            429 => crate::settings::ModelConnectionErrorKind::RateLimited,
            500..=599 => crate::settings::ModelConnectionErrorKind::ProviderUnavailable,
            _ => crate::settings::ModelConnectionErrorKind::HttpStatus,
        };
        return Err(record_vision_failure(
            &call_id,
            &context,
            &settings.model,
            String::new(),
            &started_at,
            started,
            None,
            kind.clone(),
            format!("视觉模型请求失败（{kind:?}，HTTP {status}）"),
        ));
    }
    let mut response = response;
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                    return Err(record_vision_failure(
                        &call_id,
                        &context,
                        &settings.model,
                        String::new(),
                        &started_at,
                        started,
                        None,
                        crate::settings::ModelConnectionErrorKind::Protocol,
                        "视觉模型响应超过大小限制（Protocol）",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(error) => {
                let kind = if error.is_timeout() {
                    crate::settings::ModelConnectionErrorKind::Timeout
                } else {
                    crate::settings::ModelConnectionErrorKind::Network
                };
                return Err(record_vision_failure(
                    &call_id,
                    &context,
                    &settings.model,
                    String::new(),
                    &started_at,
                    started,
                    None,
                    kind.clone(),
                    format!("读取视觉模型响应失败（{kind:?}）"),
                ));
            }
        }
    }
    let decoded: OpenAiResponse = match serde_json::from_slice(&bytes) {
        Ok(decoded) => decoded,
        Err(_) => {
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                String::new(),
                &started_at,
                started,
                None,
                crate::settings::ModelConnectionErrorKind::Protocol,
                "视觉模型响应不符合 OpenAI Compatible 协议",
            ));
        }
    };
    let usage = Some(crate::cost_ledger::ProviderUsage {
        input_tokens: decoded.usage.prompt_tokens,
        output_tokens: decoded.usage.completion_tokens,
        total_tokens: decoded.usage.total_tokens,
        cached_input_tokens: None,
    });
    let content = match decoded
        .choices
        .first()
        .map(|choice| choice.message.content.as_str())
    {
        Some(content) => content,
        None => {
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                decoded.id,
                &started_at,
                started,
                usage,
                crate::settings::ModelConnectionErrorKind::Protocol,
                "视觉模型响应缺少正文（Protocol）",
            ));
        }
    };
    let protocol: VisionProtocolResponse = match serde_json::from_str(content) {
        Ok(protocol) => protocol,
        Err(_) => {
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                decoded.id,
                &started_at,
                started,
                usage,
                crate::settings::ModelConnectionErrorKind::Protocol,
                "视觉模型正文不是约定的结构化 JSON",
            ));
        }
    };
    if !protocol.image_detected {
        return Err(record_vision_failure(
            &call_id,
            &context,
            &settings.model,
            decoded.id,
            &started_at,
            started,
            usage,
            crate::settings::ModelConnectionErrorKind::Protocol,
            "视觉模型未确认接收到图片输入",
        ));
    }
    if !capability_test {
        let expected = criteria
            .iter()
            .map(|(criterion_index, _)| *criterion_index)
            .collect::<BTreeSet<_>>();
        let actual = protocol
            .criteria
            .iter()
            .map(|result| result.criterion_index)
            .collect::<BTreeSet<_>>();
        if expected != actual || actual.len() != protocol.criteria.len() {
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                decoded.id,
                &started_at,
                started,
                usage,
                crate::settings::ModelConnectionErrorKind::Protocol,
                "视觉模型没有返回完整且唯一的逐验收项结论",
            ));
        }
        if protocol.criteria.iter().any(|result| {
            result.summary.trim().is_empty()
                || result.status == crate::project::VisualReviewStatus::Conflict
        }) {
            return Err(record_vision_failure(
                &call_id,
                &context,
                &settings.model,
                decoded.id,
                &started_at,
                started,
                usage,
                crate::settings::ModelConnectionErrorKind::Protocol,
                "视觉模型逐验收项结论缺少摘要或使用了后端专用状态",
            ));
        }
    }
    let call = metadata(
        call_id,
        context,
        settings.model.clone(),
        decoded.id,
        started_at,
        started.elapsed().as_millis() as u64,
        usage,
        String::new(),
    );
    crate::cost_ledger::record_metadata_best_effort(&call);
    Ok((protocol, call))
}

pub(crate) async fn review_task_images(
    project: &crate::project::Project,
    task: &crate::project::Subtask,
    criterion_indexes: &[u32],
    mut context: crate::cost_ledger::ModelCallContext,
) -> Result<VisionReviewResult, String> {
    if !project.vision_review_enabled {
        return Err("项目未启用视觉模型辅助".to_string());
    }
    let snapshot = crate::settings::begin_vision_request()?;
    let images = collect_visual_evidence_loaded(&project.project_path, task, &snapshot.settings)?;
    if images.is_empty() {
        return Err("任务没有明确声明 PNG、JPEG 或 WebP 视觉证据".to_string());
    }
    context.purpose = Some(crate::cost_ledger::ModelCallPurpose::VisionReview);
    let criteria = criteria_for_indexes(task, criterion_indexes)?;
    let (protocol, metadata) = call_vision_model(
        &snapshot.settings,
        &snapshot.api_key,
        &images,
        &criteria,
        false,
        context,
    )
    .await
    .map_err(|error| error.message)?;
    Ok(VisionReviewResult {
        model: snapshot.settings.model.clone(),
        latency_ms: metadata.elapsed_ms,
        criteria: protocol.criteria,
        evidence: images.into_iter().map(|item| item.reference).collect(),
    })
}

pub(crate) async fn test_connection() -> crate::settings::ConnectionTestResult {
    let started = Instant::now();
    let snapshot = match crate::settings::begin_vision_request() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return crate::settings::ConnectionTestResult {
                success: false,
                target: crate::settings::ModelConnectionTarget::VisionModel,
                model: String::new(),
                latency_ms: 0,
                error_kind: Some(crate::settings::ModelConnectionErrorKind::MissingSecret),
                message: error,
            };
        }
    };
    let image = LoadedVisualEvidence {
        reference: crate::project::VisualEvidenceReference {
            path: "embedded-capability-test.png".to_string(),
            sha256: format!("sha256:{:x}", Sha256::digest(CAPABILITY_TEST_PNG)),
            mime: "image/png".to_string(),
            size_bytes: CAPABILITY_TEST_PNG.len() as u64,
        },
        bytes: CAPABILITY_TEST_PNG.to_vec(),
    };
    match call_vision_model(
        &snapshot.settings,
        &snapshot.api_key,
        &[image],
        &[],
        true,
        crate::cost_ledger::ModelCallContext::default(),
    )
    .await
    {
        Ok(_) => crate::settings::ConnectionTestResult {
            success: true,
            target: crate::settings::ModelConnectionTarget::VisionModel,
            model: snapshot.settings.model.clone(),
            latency_ms: started.elapsed().as_millis() as u64,
            error_kind: None,
            message: "视觉模型已真实接收微型 PNG".to_string(),
        },
        Err(error) => crate::settings::ConnectionTestResult {
            success: false,
            target: crate::settings::ModelConnectionTarget::VisionModel,
            model: snapshot.settings.model.clone(),
            latency_ms: started.elapsed().as_millis() as u64,
            error_kind: Some(error.kind),
            message: error.message,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn one_shot_server(
        status: &str,
        body: Vec<u8>,
        body_delay: Duration,
    ) -> Result<(String, tokio::task::JoinHandle<Result<(), String>>), String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut request = vec![0u8; 16 * 1024];
            socket
                .read(&mut request)
                .await
                .map_err(|error| error.to_string())?;
            socket
                .write_all(headers.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            socket.flush().await.map_err(|error| error.to_string())?;
            if !body_delay.is_zero() {
                tokio::time::sleep(body_delay).await;
            }
            let _ = socket.write_all(&body).await;
            Ok(())
        });
        Ok((format!("http://{address}/v1/chat/completions"), handle))
    }

    fn vision_settings(request_url: String) -> crate::settings::VisionModelSettings {
        crate::settings::VisionModelSettings {
            enabled: true,
            request_url,
            model: "vision-test".to_string(),
            timeout_secs: 2,
            ..Default::default()
        }
    }

    fn capability_image() -> LoadedVisualEvidence {
        LoadedVisualEvidence {
            reference: crate::project::VisualEvidenceReference {
                path: "embedded-capability-test.png".to_string(),
                sha256: format!("sha256:{:x}", Sha256::digest(CAPABILITY_TEST_PNG)),
                mime: "image/png".to_string(),
                size_bytes: CAPABILITY_TEST_PNG.len() as u64,
            },
            bytes: CAPABILITY_TEST_PNG.to_vec(),
        }
    }

    fn openai_body(image_detected: bool, content_override: Option<&str>) -> Vec<u8> {
        let content = content_override.map(str::to_string).unwrap_or_else(|| {
            serde_json::json!({"image_detected": image_detected, "criteria": []}).to_string()
        });
        serde_json::to_vec(&serde_json::json!({
            "id": "vision-response-1",
            "choices": [{"message": {"content": content}}],
            "usage": {"prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6}
        }))
        .expect("测试响应必须可序列化")
    }

    #[tokio::test]
    async fn capability_call_requires_real_image_detection() -> Result<(), String> {
        let (url, server) =
            one_shot_server("200 OK", openai_body(true, None), Duration::ZERO).await?;
        let result = call_vision_model(
            &vision_settings(url),
            "vision-secret",
            &[capability_image()],
            &[],
            true,
            Default::default(),
        )
        .await
        .map_err(|error| error.message)?;
        assert!(result.0.image_detected);
        assert_eq!(result.1.usage.and_then(|usage| usage.total_tokens), Some(6));
        server.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn capability_call_classifies_authentication_without_reading_error_body(
    ) -> Result<(), String> {
        let (url, server) = one_shot_server(
            "401 Unauthorized",
            b"secret provider detail".to_vec(),
            Duration::ZERO,
        )
        .await?;
        let error = call_vision_model(
            &vision_settings(url),
            "vision-secret",
            &[capability_image()],
            &[],
            true,
            Default::default(),
        )
        .await
        .expect_err("认证失败必须保留结构化分类");
        assert_eq!(
            error.kind,
            crate::settings::ModelConnectionErrorKind::Authentication
        );
        assert!(!error.message.contains("secret provider detail"));
        server.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn capability_call_classifies_timeout_while_waiting_for_body() -> Result<(), String> {
        let (url, server) = one_shot_server(
            "200 OK",
            openai_body(true, None),
            Duration::from_millis(1_250),
        )
        .await?;
        let mut settings = vision_settings(url);
        settings.timeout_secs = 1;
        let error = call_vision_model(
            &settings,
            "vision-secret",
            &[capability_image()],
            &[],
            true,
            Default::default(),
        )
        .await
        .expect_err("停滞正文必须超时");
        assert_eq!(
            error.kind,
            crate::settings::ModelConnectionErrorKind::Timeout
        );
        server.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn capability_call_rejects_missing_image_capability_and_invalid_protocol(
    ) -> Result<(), String> {
        for (body, expected_message) in [
            (openai_body(false, None), "未确认接收到图片"),
            (openai_body(true, Some("not-json")), "结构化 JSON"),
        ] {
            let (url, server) = one_shot_server("200 OK", body, Duration::ZERO).await?;
            let error = call_vision_model(
                &vision_settings(url),
                "vision-secret",
                &[capability_image()],
                &[],
                true,
                Default::default(),
            )
            .await
            .expect_err("无视觉能力或协议错误必须失败");
            assert_eq!(
                error.kind,
                crate::settings::ModelConnectionErrorKind::Protocol
            );
            assert!(error.message.contains(expected_message));
            server.await.map_err(|error| error.to_string())??;
        }
        Ok(())
    }

    #[tokio::test]
    async fn capability_call_stops_when_response_crosses_size_limit() -> Result<(), String> {
        let (url, server) =
            one_shot_server("200 OK", vec![b'x'; MAX_RESPONSE_BYTES + 1], Duration::ZERO).await?;
        let error = call_vision_model(
            &vision_settings(url),
            "vision-secret",
            &[capability_image()],
            &[],
            true,
            Default::default(),
        )
        .await
        .expect_err("超大正文必须在分块读取时失败");
        assert_eq!(
            error.kind,
            crate::settings::ModelConnectionErrorKind::Protocol
        );
        assert!(error.message.contains("大小限制"));
        server.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[test]
    fn collector_rejects_symlink_escape_and_records_only_metadata() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("metheus-vision-{}", uuid::Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("metheus-vision-out-{}.png", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        std::fs::write(&outside, CAPABILITY_TEST_PNG).map_err(|error| error.to_string())?;
        let link = root.join("escape.png");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).map_err(|error| error.to_string())?;
        let task = crate::project::Subtask {
            evidence_files: vec!["escape.png".to_string()],
            ..Default::default()
        };
        #[cfg(unix)]
        assert!(collect_visual_evidence(
            root.to_str().unwrap_or_default(),
            &task,
            &crate::settings::VisionModelSettings::default(),
        )
        .unwrap_err()
        .contains("越过项目根目录"));
        let _ = std::fs::remove_file(outside);
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn collector_validates_mime_hash_and_size() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("metheus-vision-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("shots")).map_err(|error| error.to_string())?;
        std::fs::write(root.join("shots/view.png"), CAPABILITY_TEST_PNG)
            .map_err(|error| error.to_string())?;
        let task = crate::project::Subtask {
            expected_artifacts: vec!["shots/view.png".to_string()],
            ..Default::default()
        };
        let evidence = collect_visual_evidence(
            root.to_str().unwrap_or_default(),
            &task,
            &crate::settings::VisionModelSettings::default(),
        )?;
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].mime, "image/png");
        assert_eq!(evidence[0].size_bytes, CAPABILITY_TEST_PNG.len() as u64);
        assert!(evidence[0].sha256.starts_with("sha256:"));
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn evidence_fingerprint_rejects_undeclared_and_changed_images() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("metheus-vision-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("shots")).map_err(|error| error.to_string())?;
        std::fs::write(root.join("shots/view.png"), CAPABILITY_TEST_PNG)
            .map_err(|error| error.to_string())?;
        std::fs::write(root.join("shots/other.png"), CAPABILITY_TEST_PNG)
            .map_err(|error| error.to_string())?;
        let task = crate::project::Subtask {
            evidence_files: vec!["shots/view.png".to_string()],
            ..Default::default()
        };
        let evidence = collect_visual_evidence(
            root.to_str().unwrap_or_default(),
            &task,
            &crate::settings::VisionModelSettings::default(),
        )?;
        validate_visual_evidence_fingerprints(root.to_str().unwrap_or_default(), &task, &evidence)?;

        let mut undeclared = evidence.clone();
        undeclared[0].path = "shots/other.png".to_string();
        assert!(validate_visual_evidence_fingerprints(
            root.to_str().unwrap_or_default(),
            &task,
            &undeclared,
        )
        .unwrap_err()
        .contains("未声明"));

        let mut changed = CAPABILITY_TEST_PNG.to_vec();
        changed.push(0);
        std::fs::write(root.join("shots/view.png"), changed).map_err(|error| error.to_string())?;
        let error = validate_visual_evidence_fingerprints(
            root.to_str().unwrap_or_default(),
            &task,
            &evidence,
        )
        .unwrap_err();
        assert!(error.contains("fingerprint"));
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn project_switch_blocks_before_settings_or_network_access() {
        let project = crate::project::Project::new("vision-disabled");
        let task = crate::project::Subtask::default();
        let error = review_task_images(&project, &task, &[1], Default::default())
            .await
            .unwrap_err();
        assert!(error.contains("项目未启用"));
    }

    #[test]
    fn visual_outcomes_remain_auxiliary_and_conflicts_are_explicit() {
        use crate::project::{AcceptanceStatus, VisualReviewStatus};

        assert_eq!(
            reconcile_visual_status(
                &AcceptanceStatus::AiProvisionallySatisfied,
                VisualReviewStatus::Unsatisfied,
            ),
            VisualReviewStatus::Conflict
        );
        assert_eq!(
            reconcile_visual_status(
                &AcceptanceStatus::DeferredHumanReview,
                VisualReviewStatus::Satisfied,
            ),
            VisualReviewStatus::Satisfied
        );
        assert_eq!(
            reconcile_visual_status(
                &AcceptanceStatus::DeferredHumanReview,
                VisualReviewStatus::EvidenceInsufficient,
            ),
            VisualReviewStatus::EvidenceInsufficient
        );
    }

    #[test]
    fn visual_criteria_selection_rejects_missing_and_duplicate_indexes() {
        let task = crate::project::Subtask {
            acceptance_criteria: vec!["按钮可点击".to_string()],
            ..Default::default()
        };
        assert!(criteria_for_indexes(&task, &[0]).is_err());
        assert!(criteria_for_indexes(&task, &[2]).is_err());
        assert!(criteria_for_indexes(&task, &[1, 1]).is_err());
        assert_eq!(
            criteria_for_indexes(&task, &[1]).unwrap(),
            vec![(1, "按钮可点击".to_string())]
        );
    }
}
