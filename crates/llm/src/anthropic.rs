//! Adapter for the Anthropic Messages API.
use crate::{Completion, CompletionRequest, LlmError, OnDelta, Provider};
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

/// Reads the Messages API event stream (`data: {json}` lines) into text deltas and token counts.
#[derive(Default)]
struct SseState {
    pending: Vec<u8>,
    text: String,
    input_tokens: u64,
    output_tokens: u64,
}

impl SseState {
    /// Bytes may split a line (or a character) anywhere, so only complete lines are decoded.
    fn feed(&mut self, bytes: &[u8], on_delta: OnDelta<'_>) -> Result<(), LlmError> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            match v["type"].as_str() {
                Some("message_start") => {
                    self.input_tokens = v["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0)
                }
                Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                    if let Some(t) = v["delta"]["text"].as_str() {
                        self.text.push_str(t);
                        on_delta(t);
                    }
                }
                Some("message_delta") => {
                    self.output_tokens = v["usage"]["output_tokens"].as_u64().unwrap_or(0)
                }
                // Type only: the message can echo prompt text.
                Some("error") => {
                    let kind = v["error"]["type"].as_str().unwrap_or("unknown");
                    return Err(LlmError::Provider(format!("stream error: {kind}")));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for Anthropic {
    async fn stream(
        &self,
        req: CompletionRequest,
        on_delta: OnDelta<'_>,
    ) -> Result<Completion, LlmError> {
        let mut b = body(&req);
        b["stream"] = json!(true);
        let mut resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&b)
            .send()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(LlmError::Provider(format!("http {}", resp.status())));
        }
        let mut st = SseState::default();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| LlmError::Provider(e.to_string()))?
        {
            st.feed(&chunk, on_delta)?;
        }
        Ok(Completion {
            text: st.text,
            input_tokens: st.input_tokens,
            output_tokens: st.output_tokens,
        })
    }

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
    fn stream_collects_deltas_and_usage_across_split_chunks() {
        let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":25,\"output_tokens\":1}}}\n\n\
                   data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Xin \"}}\n\n\
                   data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"chào ế\"}}\n\n\
                   data: {\"type\":\"ping\"}\n\n\
                   data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n";
        let mut st = SseState::default();
        let mut seen: Vec<String> = vec![];
        // Three-byte pieces cut lines and multi-byte characters in arbitrary places.
        for piece in sse.as_bytes().chunks(3) {
            st.feed(piece, &mut |t| seen.push(t.to_string())).unwrap();
        }
        assert_eq!(seen, ["Xin ", "chào ế"]);
        assert_eq!(
            (st.text.as_str(), st.input_tokens, st.output_tokens),
            ("Xin chào ế", 25, 7)
        );
    }

    #[test]
    fn stream_error_event_fails_without_echoing_its_message() {
        let mut st = SseState::default();
        let e = st
            .feed(
                b"data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"secret prompt text\"}}\n",
                &mut |_| {},
            )
            .unwrap_err()
            .to_string();
        assert_eq!(e, "provider error: stream error: overloaded_error");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(matches!(parse("{}"), Err(LlmError::Provider(_))));
    }
}
