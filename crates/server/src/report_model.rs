//! `ReportModel` backed by the llm router: four tasks per meeting, one `llm_usage` row per call.
use crate::config::Llm;
use anyhow::{anyhow, Result};
use recapz_core::report_job::{ActionItemData, ReportData, ReportModel, UsageData};
use recapz_llm::{
    anthropic::Anthropic,
    openai_compat::OpenAiCompat,
    report::{self, generate, ActionItems, Chapters, Decisions, Summary},
    LlmError, Price, Route, Routed, Router, Task,
};
use std::sync::Arc;

fn usage(task: &str, calls: &[Routed]) -> Vec<UsageData> {
    calls
        .iter()
        .map(|c| UsageData {
            task: task.into(),
            provider: c.provider.clone(),
            model: c.model.clone(),
            input_tokens: c.completion.input_tokens,
            output_tokens: c.completion.output_tokens,
            cost_micro_usd: c.cost_micro_usd,
        })
        .collect()
}

pub async fn generate_report(
    router: &Router,
    model: &str,
    transcript: &str,
) -> Result<ReportData, LlmError> {
    let s = generate::<Summary>(router, Task::Summary, report::SUMMARY, transcript).await?;
    let d = generate::<Decisions>(router, Task::Decisions, report::DECISIONS, transcript).await?;
    let c = generate::<Chapters>(router, Task::Chapters, report::CHAPTERS, transcript).await?;
    let a = generate::<ActionItems>(router, Task::ActionItems, report::ACTION_ITEMS, transcript)
        .await?;
    let mut u = usage("summary", &s.calls);
    u.extend(usage("decisions", &d.calls));
    u.extend(usage("chapters", &c.calls));
    u.extend(usage("action_items", &a.calls));
    Ok(ReportData {
        summary: s.value.summary,
        decisions: d.value.decisions,
        open_questions: d.value.open_questions,
        chapters: c
            .value
            .chapters
            .into_iter()
            .map(|c| (c.start_ms, c.title))
            .collect(),
        action_items: a
            .value
            .action_items
            .into_iter()
            .map(|i| ActionItemData {
                text: i.text,
                due: i.due,
                source_ms: i.source_ms,
            })
            .collect(),
        model: model.to_string(),
        prompt_version: report::PROMPT_VERSION,
        usage: u,
    })
}

pub struct LlmReportModel {
    router: Router,
    model: String,
    rt: tokio::runtime::Handle,
}

/// One route shared by every task, including Ask. `None` when no provider is configured.
pub fn router_from_config(cfg: &Llm) -> Result<Option<Router>> {
    if cfg.provider.is_empty() {
        return Ok(None);
    }
    let p: Arc<dyn recapz_llm::Provider> = match cfg.provider.as_str() {
        "anthropic" if cfg.base_url.is_empty() => Arc::new(Anthropic::new(&cfg.api_key)),
        "anthropic" => Arc::new(Anthropic::with_base_url(&cfg.api_key, &cfg.base_url)),
        "openai" if !cfg.base_url.is_empty() => {
            Arc::new(OpenAiCompat::new(&cfg.base_url, &cfg.api_key))
        }
        "openai" => return Err(anyhow!("llm.base_url is required for provider 'openai'")),
        other => return Err(anyhow!("unknown llm.provider '{other}'")),
    };
    if cfg.model.is_empty() {
        return Err(anyhow!("llm.model is required"));
    }
    let route = Route {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        max_tokens: if cfg.max_tokens == 0 {
            2048
        } else {
            cfg.max_tokens
        },
        price: Price {
            input: cfg.price_input,
            output: cfg.price_output,
        },
    };
    let mut router = Router::new().provider(&cfg.provider, p);
    for t in [
        Task::Summary,
        Task::Decisions,
        Task::Chapters,
        Task::ActionItems,
        Task::AskAnswer,
        Task::LiveAnswer,
    ] {
        router = router.route(t, route.clone());
    }
    Ok(Some(router))
}

impl LlmReportModel {
    /// `None` when no provider is configured.
    pub fn from_config(cfg: &Llm, rt: tokio::runtime::Handle) -> Result<Option<Self>> {
        Ok(router_from_config(cfg)?.map(|router| Self {
            router,
            model: cfg.model.clone(),
            rt,
        }))
    }
}

impl ReportModel for LlmReportModel {
    /// Blocks on the async calls, so it must run on a blocking thread (the job runner does).
    fn generate(&self, transcript: &str) -> Result<ReportData> {
        Ok(self
            .rt
            .block_on(generate_report(&self.router, &self.model, transcript))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recapz_llm::{stub::Stub, Completion};

    fn ok(text: &str, i: u64, o: u64) -> Result<Completion, LlmError> {
        Ok(Completion {
            text: text.into(),
            input_tokens: i,
            output_tokens: o,
        })
    }

    fn router(stub: Arc<Stub>) -> Router {
        let r = Route {
            provider: "p".into(),
            model: "m".into(),
            max_tokens: 9,
            price: Price {
                input: 1_000_000,
                output: 0,
            },
        };
        [
            Task::Summary,
            Task::Decisions,
            Task::Chapters,
            Task::ActionItems,
        ]
        .into_iter()
        .fold(Router::new().provider("p", stub), |rt, t| {
            rt.route(t, r.clone())
        })
    }

    #[tokio::test]
    async fn maps_four_tasks_into_report_data_with_usage() {
        let stub = Arc::new(Stub::new(vec![
            ok(r#"{"summary":"S"}"#, 10, 1),
            ok(r#"{"decisions":["d"],"open_questions":["q"]}"#, 20, 1),
            ok(r#"{"chapters":[{"start_ms":5,"title":"T"}]}"#, 30, 1),
            ok(
                r#"{"action_items":[{"text":"x","owner":"Lan","due":"2026-12-01","source_ms":7}]}"#,
                40,
                1,
            ),
        ]));
        let d = generate_report(&router(stub.clone()), "m", "tr")
            .await
            .unwrap();
        assert_eq!(
            (d.summary.as_str(), d.decisions, d.open_questions),
            ("S", vec!["d".to_string()], vec!["q".to_string()])
        );
        assert_eq!(d.chapters, vec![(5, "T".to_string())]);
        assert_eq!(
            (
                d.action_items[0].text.as_str(),
                d.action_items[0].due.as_deref(),
                d.action_items[0].source_ms
            ),
            ("x", Some("2026-12-01"), Some(7))
        );
        let u: Vec<(&str, u64)> = d
            .usage
            .iter()
            .map(|u| (u.task.as_str(), u.cost_micro_usd))
            .collect();
        assert_eq!(
            u,
            vec![
                ("summary", 10),
                ("decisions", 20),
                ("chapters", 30),
                ("action_items", 40)
            ]
        );
        assert_eq!(stub.requests().len(), 4);
        assert!(stub.requests().iter().all(|r| r.prompt == "tr"));
    }

    #[tokio::test]
    async fn one_failed_task_fails_the_report() {
        let stub = Arc::new(Stub::new(vec![
            ok(r#"{"summary":"S"}"#, 1, 1),
            Err(LlmError::Provider("down".into())),
        ]));
        assert!(generate_report(&router(stub), "m", "tr").await.is_err());
    }

    #[test]
    fn config_checks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let h = rt.handle().clone();
        let mut c = Llm::default();
        assert!(LlmReportModel::from_config(&c, h.clone())
            .unwrap()
            .is_none());
        c.provider = "openai".into();
        c.model = "m".into();
        assert!(
            LlmReportModel::from_config(&c, h.clone()).is_err(),
            "openai needs base_url"
        );
        c.base_url = "http://localhost:8000/v1".into();
        assert!(LlmReportModel::from_config(&c, h.clone())
            .unwrap()
            .is_some());
        c.provider = "nope".into();
        assert!(LlmReportModel::from_config(&c, h).is_err());
    }
}
