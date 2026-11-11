//! Adapter for the Anthropic Messages API.
use crate::{Completion, CompletionRequest, LlmError, Provider};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct Anthropic {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl Anthropic {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, "https://api.anthropic.com")
    }

    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }
}

fn body(req: &CompletionRequest) -> Value {
    json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "system": req.system,
        "messages": [{"role": "user", "content": req.prompt}],
    })
}

#[derive(Deserialize)]
struct Resp {
    content: Vec<Block>,
    usage: Usage,
}
#[derive(Deserialize)]
struct Block {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}
#[derive(Deserialize)]
struct Usage {
    input_tokens: u64,
    output_tokens: u64,
}

fn parse(raw: &str) -> Result<Completion, LlmError> {
    let r: Resp =
        serde_json::from_str(raw).map_err(|e| LlmError::Provider(format!("bad response: {e}")))?;
    let text: String = r
        .content
        .iter()
        .filter(|b| b.kind == "text")
        .map(|b| b.text.as_str())
        .collect();
    Ok(Completion {
        text,
        input_tokens: r.usage.input_tokens,
        output_tokens: r.usage.output_tokens,
    })
}

#[async_trait]
impl Provider for Anthropic {
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError> {
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body(&req))
            .send()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?;
        let status = resp.status();
        let raw = resp
            .text()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?;
        if !status.is_success() {
            // Status only: the body can echo prompt text, which must stay out of errors and logs.
            return Err(LlmError::Provider(format!("http {status}")));
        }
        parse(&raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_body_puts_instructions_in_system() {
        let b = body(&CompletionRequest {
            model: "m".into(),
            max_tokens: 50,
            system: "be brief".into(),
            prompt: "hi".into(),
        });
        assert_eq!(
            b,
            json!({"model":"m","max_tokens":50,"system":"be brief","messages":[{"role":"user","content":"hi"}]})
        );
    }

    #[test]
    fn parse_joins_text_blocks_and_reads_usage() {
        let c = parse(
            r#"{"content":[{"type":"text","text":"a"},{"type":"tool_use"},{"type":"text","text":"b"}],
                "usage":{"input_tokens":12,"output_tokens":3}}"#,
        )
        .unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("ab", 12, 3)
        );
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(parse("{}"), Err(LlmError::Provider(_))));
    }
}
