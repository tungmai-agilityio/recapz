//! Scripted provider for tests: returns queued results and records requests.
use crate::{Completion, CompletionRequest, LlmError, Provider};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Mutex;

pub struct Stub {
    results: Mutex<VecDeque<Result<Completion, LlmError>>>,
    seen: Mutex<Vec<CompletionRequest>>,
}

impl Stub {
    pub fn new(results: Vec<Result<Completion, LlmError>>) -> Self {
        Self {
            results: Mutex::new(results.into()),
            seen: Mutex::new(vec![]),
        }
    }

    /// Reply with plain text and zero usage, once per string.
    pub fn texts(texts: &[&str]) -> Self {
        Self::new(
            texts
                .iter()
                .map(|t| {
                    Ok(Completion {
                        text: t.to_string(),
                        input_tokens: 0,
                        output_tokens: 0,
                    })
                })
                .collect(),
        )
    }

    pub fn requests(&self) -> Vec<CompletionRequest> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl Provider for Stub {
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError> {
        self.seen.lock().unwrap().push(req);
        self.results
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(LlmError::Provider("stub exhausted".into())))
    }
}
