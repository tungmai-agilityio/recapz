//! Adapter for OpenAI-compatible Chat Completions servers (hosted or a local server).
use crate::{Completion, CompletionRequest, LlmError, Provider};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

pub struct OpenAiCompat {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl OpenAiCompat {
    /// `base_url` is the API root without the `/chat/completions` suffix.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("http client"),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }
}

fn body(req: &CompletionRequest) -> Value {
    json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "messages": [
            {"role": "system", "content": req.system},
            {"role": "user", "content": req.prompt},
        ],
    })
}

#[derive(Deserialize)]
struct Resp {
    choices: Vec<Choice>,
    /// Some local servers omit usage; counts then default to 0.
    #[serde(default)]
    usage: Usage,
}
#[derive(Deserialize)]
struct Choice {
    message: Message,
}
#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: Option<String>,
}
#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

fn parse(raw: &str) -> Result<Completion, LlmError> {
    let r: Resp =
        serde_json::from_str(raw).map_err(|e| LlmError::Provider(format!("bad response: {e}")))?;
    let text = r
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message.content)
        .ok_or_else(|| LlmError::Provider("no choices".into()))?;
    Ok(Completion {
        text,
        input_tokens: r.usage.prompt_tokens,
        output_tokens: r.usage.completion_tokens,
    })
}

#[async_trait]
impl Provider for OpenAiCompat {
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError> {
        let mut rb = self
            .http
            .post(format!("{}/chat/completions", self.base_url));
        if !self.api_key.is_empty() {
            rb = rb.bearer_auth(&self.api_key);
        }
        let resp = rb
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
    fn request_body_has_system_then_user_message() {
        let b = body(&CompletionRequest {
            model: "m".into(),
            max_tokens: 50,
            system: "be brief".into(),
            prompt: "hi".into(),
        });
        assert_eq!(
            b,
            json!({"model":"m","max_tokens":50,"messages":[
                {"role":"system","content":"be brief"},{"role":"user","content":"hi"}]})
        );
    }

    #[test]
    fn parse_reads_first_choice_and_usage() {
        let c = parse(
            r#"{"choices":[{"message":{"content":"a"}},{"message":{"content":"b"}}],
                "usage":{"prompt_tokens":7,"completion_tokens":2}}"#,
        )
        .unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("a", 7, 2)
        );
    }

    #[test]
    fn parse_defaults_missing_usage_to_zero() {
        let c = parse(r#"{"choices":[{"message":{"content":"a"}}]}"#).unwrap();
        assert_eq!(
            (c.text.as_str(), c.input_tokens, c.output_tokens),
            ("a", 0, 0)
        );
        let c = parse(r#"{"choices":[{"message":{"content":"a"}}],"usage":{"prompt_tokens":4}}"#)
            .unwrap();
        assert_eq!((c.input_tokens, c.output_tokens), (4, 0));
    }

    #[test]
    fn parse_rejects_empty_choices_and_null_content() {
        let usage = r#""usage":{"prompt_tokens":1,"completion_tokens":1}"#;
        assert!(parse(&format!(r#"{{"choices":[],{usage}}}"#)).is_err());
        assert!(parse(&format!(
            r#"{{"choices":[{{"message":{{"content":null}}}}],{usage}}}"#
        ))
        .is_err());
    }
}
