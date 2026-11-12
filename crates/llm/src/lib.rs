//! Language-model access: a `Provider` trait, a per-task `Router` and cost accounting.
//! Only transcript text is ever sent to a provider.
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub mod anthropic;
pub mod openai_compat;
pub mod report;
pub mod stub;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Task {
    Chapters,
    ActionItems,
    Decisions,
    Summary,
    AskAnswer,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub max_tokens: u32,
    /// Fixed instructions first, so provider prompt caching can apply.
    pub system: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("no route configured for {0:?}")]
    NoRoute(Task),
    #[error("provider '{0}' is not configured")]
    UnknownProvider(String),
    #[error("provider error: {0}")]
    Provider(String),
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, LlmError>;
}

/// Price in micro-USD per one million tokens.
#[derive(Debug, Clone, Copy, Default)]
pub struct Price {
    pub input: u64,
    pub output: u64,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub price: Price,
}

/// A finished call, shaped for one `llm_usage` row.
#[derive(Debug, Clone)]
pub struct Routed {
    pub completion: Completion,
    pub provider: String,
    pub model: String,
    pub cost_micro_usd: u64,
}

#[derive(Default)]
pub struct Router {
    providers: HashMap<String, Arc<dyn Provider>>,
    routes: HashMap<Task, Route>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn provider(mut self, name: &str, p: Arc<dyn Provider>) -> Self {
        self.providers.insert(name.to_string(), p);
        self
    }

    pub fn route(mut self, task: Task, route: Route) -> Self {
        self.routes.insert(task, route);
        self
    }

    pub async fn run(&self, task: Task, system: &str, prompt: &str) -> Result<Routed, LlmError> {
        let route = self.routes.get(&task).ok_or(LlmError::NoRoute(task))?;
        let p = self
            .providers
            .get(&route.provider)
            .ok_or_else(|| LlmError::UnknownProvider(route.provider.clone()))?;
        let completion = p
            .complete(CompletionRequest {
                model: route.model.clone(),
                max_tokens: route.max_tokens,
                system: system.to_string(),
                prompt: prompt.to_string(),
            })
            .await?;
        let cost_micro_usd = cost(route.price, &completion);
        Ok(Routed {
            completion,
            provider: route.provider.clone(),
            model: route.model.clone(),
            cost_micro_usd,
        })
    }
}

pub fn cost(price: Price, c: &Completion) -> u64 {
    (c.input_tokens * price.input + c.output_tokens * price.output).div_ceil(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stub::Stub;

    fn route(provider: &str) -> Route {
        Route {
            provider: provider.into(),
            model: "m1".into(),
            max_tokens: 100,
            price: Price {
                input: 3_000_000,
                output: 15_000_000,
            },
        }
    }

    #[test]
    fn cost_rounds_up_to_a_micro_usd() {
        let c = Completion {
            text: String::new(),
            input_tokens: 1_000,
            output_tokens: 1,
        };
        // 1000 * 3_000_000 + 1 * 15_000_000 = 3_015_000_000 / 1e6 = 3015
        assert_eq!(
            cost(
                Price {
                    input: 3_000_000,
                    output: 15_000_000
                },
                &c
            ),
            3015
        );
        let tiny = Completion {
            text: String::new(),
            input_tokens: 1,
            output_tokens: 0,
        };
        assert_eq!(
            cost(
                Price {
                    input: 1,
                    output: 0
                },
                &tiny
            ),
            1
        );
    }

    #[tokio::test]
    async fn routes_task_to_its_provider_and_prices_it() {
        let stub = Arc::new(Stub::new(vec![Ok(Completion {
            text: "ok".into(),
            input_tokens: 2_000,
            output_tokens: 100,
        })]));
        let r = Router::new()
            .provider("p", stub.clone())
            .route(Task::Summary, route("p"));
        let out = r.run(Task::Summary, "sys", "hello").await.unwrap();
        assert_eq!(out.completion.text, "ok");
        assert_eq!(out.model, "m1");
        assert_eq!(out.cost_micro_usd, 7_500); // 2000*3 + 100*15 = 7500 micro-USD
        let seen = stub.requests();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            (seen[0].system.as_str(), seen[0].prompt.as_str()),
            ("sys", "hello")
        );
        assert_eq!(seen[0].max_tokens, 100);
    }

    #[tokio::test]
    async fn missing_route_or_provider_is_an_error() {
        let r = Router::new().route(Task::Chapters, route("nope"));
        assert!(matches!(
            r.run(Task::Summary, "", "").await,
            Err(LlmError::NoRoute(Task::Summary))
        ));
        assert!(
            matches!(r.run(Task::Chapters, "", "").await, Err(LlmError::UnknownProvider(p)) if p == "nope")
        );
    }
}
