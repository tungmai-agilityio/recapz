//! Picks the text embedder named in config. Ask and the index job must use the same one.
use crate::config::Llm;
use recapz_core::embed::{Embedder, HashEmbedder};
use std::{sync::Arc, time::Duration};

const HOSTED_TIMEOUT: Duration = Duration::from_secs(30);

/// `hash` is the built-in word-overlap baseline. `hosted` calls `POST {base}/embeddings` on the
/// OpenAI-compatible server in `[llm]`; it sends transcript text, documents marked "May be sent to
/// the language model" and a member's question, never audio. Must run inside the tokio runtime.
pub fn from_config(name: &str, llm: &Llm) -> anyhow::Result<Arc<dyn Embedder>> {
    match name {
        "hash" => Ok(Arc::new(HashEmbedder)),
        "hosted" => Ok(Arc::new(HostedEmbedder::new(
            llm,
            HOSTED_TIMEOUT,
            tokio::runtime::Handle::current(),
        )?)),
        other => anyhow::bail!("unknown embedder {other:?}; this build has: hash, hosted"),
    }
}

/// Calls the provider's embeddings endpoint. `embed` blocks, so it must run on a blocking thread
/// (the job runner and Ask retrieval both do). Errors never carry the input text or the URL.
pub struct HostedEmbedder {
    http: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
    id: &'static str,
    rt: tokio::runtime::Handle,
}

impl HostedEmbedder {
    pub fn new(llm: &Llm, timeout: Duration, rt: tokio::runtime::Handle) -> anyhow::Result<Self> {
        let base = if llm.embedding_base_url.is_empty() {
            &llm.base_url
        } else {
            &llm.embedding_base_url
        };
        if base.is_empty() || llm.embedding_model.is_empty() {
            anyhow::bail!(
                "embedder \"hosted\" needs llm.embedding_model and llm.base_url or llm.embedding_base_url"
            );
        }
        // ponytail: the id is the model name only, so a provider swapping the model's output behind
        // the same name is not noticed; rename the model in config to force a re-index.
        let id: &'static str =
            Box::leak(format!("hosted-{}", llm.embedding_model).into_boxed_str());
        Ok(Self {
            http: reqwest::Client::builder().timeout(timeout).build()?,
            url: format!("{}/embeddings", base.trim_end_matches('/')),
            api_key: llm.api_key.clone(),
            model: llm.embedding_model.clone(),
            id,
            rt,
        })
    }

    async fn call(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut req = self
            .http
            .post(&self.url)
            .json(&serde_json::json!({ "model": self.model, "input": text }));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow::anyhow!("embedding provider timed out")
            } else {
                anyhow::anyhow!("embedding request failed")
            }
        })?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "embedding provider returned HTTP {}",
                resp.status().as_u16()
            );
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|_| anyhow::anyhow!("embedding provider sent an unreadable reply"))?;
        let bad = || anyhow::anyhow!("embedding provider sent no usable vector");
        let v: Vec<f32> = body["data"][0]["embedding"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Option<_>>()
            .ok_or_else(bad)?;
        if v.is_empty() || v.iter().any(|x| !x.is_finite()) {
            return Err(bad());
        }
        Ok(v)
    }
}

impl Embedder for HostedEmbedder {
    fn id(&self) -> &'static str {
        self.id
    }
    fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        self.rt.block_on(self.call(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
    use std::sync::Mutex;

    type Seen = Arc<Mutex<Vec<serde_json::Value>>>;

    /// Serves POST /v1/embeddings: `mode` picks ok / err / slow / empty. Returns (base_url, requests seen).
    async fn mock(mode: &'static str) -> (String, Seen) {
        let seen: Seen = Default::default();
        async fn h(
            State((mode, seen)): State<(&'static str, Seen)>,
            Json(b): Json<serde_json::Value>,
        ) -> (StatusCode, Json<serde_json::Value>) {
            seen.lock().unwrap().push(b);
            match mode {
                "err" => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json("secret provider detail".into()),
                ),
                "slow" => {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    (StatusCode::OK, Json(serde_json::json!({})))
                }
                "empty" => (StatusCode::OK, Json(serde_json::json!({ "data": [] }))),
                _ => (
                    StatusCode::OK,
                    Json(
                        serde_json::json!({ "data": [{ "index": 0, "embedding": [0.5, -1.0, 2.0] }] }),
                    ),
                ),
            }
        }
        let app = Router::new()
            .route("/v1/embeddings", post(h))
            .with_state((mode, seen.clone()));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, seen)
    }

    fn llm(base: &str) -> Llm {
        Llm {
            base_url: base.into(),
            api_key: "k".into(),
            embedding_model: "m1".into(),
            ..Default::default()
        }
    }

    async fn embed(base: &str, ms: u64, text: &'static str) -> anyhow::Result<Vec<f32>> {
        let e = HostedEmbedder::new(
            &llm(base),
            Duration::from_millis(ms),
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        tokio::task::spawn_blocking(move || e.embed(text))
            .await
            .unwrap()
    }

    #[test]
    fn known_name_loads_and_unknown_name_is_an_error() {
        let llm = Llm::default();
        assert_eq!(from_config("hash", &llm).unwrap().id(), "hash-256");
        let e = from_config("minilm", &llm).err().unwrap().to_string();
        assert_eq!(
            e,
            "unknown embedder \"minilm\"; this build has: hash, hosted"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hosted_needs_a_model_and_an_endpoint() {
        let e = from_config("hosted", &Llm::default())
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("llm.embedding_model"), "{e}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn success_returns_the_vector_and_sends_model_and_text() {
        let (base, seen) = mock("ok").await;
        assert_eq!(
            embed(&base, 2000, "hello").await.unwrap(),
            vec![0.5, -1.0, 2.0]
        );
        {
            let s = seen.lock().unwrap();
            assert_eq!(s.len(), 1);
            assert_eq!(s[0], serde_json::json!({ "model": "m1", "input": "hello" }));
        }
        let id = from_config("hosted", &llm(&base)).unwrap().id();
        assert_eq!(id, "hosted-m1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn provider_error_keeps_status_only() {
        let (base, _) = mock("err").await;
        let e = embed(&base, 2000, "private words")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(e, "embedding provider returned HTTP 500");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn timeout_is_an_error_without_url_or_text() {
        let (base, _) = mock("slow").await;
        let e = embed(&base, 150, "private words")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(e, "embedding provider timed out");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unreachable_provider_error_has_no_url() {
        let e = embed("http://127.0.0.1:1/v1", 500, "x")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(e, "embedding request failed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reply_without_a_vector_is_an_error() {
        let (base, _) = mock("empty").await;
        let e = embed(&base, 2000, "x").await.unwrap_err().to_string();
        assert_eq!(e, "embedding provider sent no usable vector");
    }
}
