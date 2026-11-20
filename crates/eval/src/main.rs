use anyhow::{bail, Context, Result};
use eval::ask::{AskReport, PASS_RATE};
use recapz_llm::{openai_compat::OpenAiCompat, Price, Route, Router, Task};
use std::sync::Arc;

const LIVE_SET: &str = include_str!("../fixtures/live-answer-set.json");

fn print(prefix: &str, r: &AskReport) -> Result<()> {
    println!("{prefix}_total {}", r.total);
    println!("{prefix}_correct {}", r.correct);
    println!("{prefix}_accuracy {:.4}", r.accuracy());
    for m in &r.misses {
        println!("miss: {m}");
    }
    if r.accuracy() < PASS_RATE {
        std::process::exit(1);
    }
    Ok(())
}

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Key, model and endpoint for the live answer eval. The key is the server's `RECAPZ_LLM_KEY`.
fn live_config(env: impl Fn(&str) -> Option<String>) -> Result<(String, String, String)> {
    let key = env("RECAPZ_LLM_KEY").context(
        "RECAPZ_LLM_KEY is not set; the live answer eval calls a model (synthetic text only)",
    )?;
    let model = env("RECAPZ_EVAL_MODEL").context("RECAPZ_EVAL_MODEL is not set")?;
    let base = env("RECAPZ_EVAL_BASE_URL").unwrap_or_else(|| DEFAULT_BASE_URL.into());
    Ok((key, model, base))
}

/// OpenAI (or any OpenAI-compatible server), configured from the environment (never printed or stored).
fn live_router() -> Result<Router> {
    let (key, model, base) = live_config(|k| std::env::var(k).ok())?;
    Ok(Router::new()
        .provider("openai", Arc::new(OpenAiCompat::new(base, key)))
        .route(
            Task::LiveAnswer,
            Route {
                provider: "openai".into(),
                model,
                // Reasoning models spend max_tokens on thinking before the reply.
                max_tokens: 8192,
                price: Price::default(),
            },
        ))
}

/// `cargo run -p eval` or `-- live [set.json]`: score live answers with a model (needs RECAPZ_LLM_KEY, RECAPZ_EVAL_MODEL; RECAPZ_EVAL_BASE_URL is optional)
/// `cargo run -p eval -- wer <reference.txt> <hypothesis.txt>`
/// `cargo run -p eval -- ask <set.json>` (retrieval only, exits 1 below the 80% pass rate)
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [cmd, set] = args.as_slice() {
        if cmd == "ask" {
            let set = serde_json::from_str(&std::fs::read_to_string(set).context("read set")?)
                .context("parse set")?;
            return print("ask", &eval::ask::run(&set)?);
        }
    }
    if args.is_empty() || (args[0] == "live" && args.len() <= 2) {
        let set = match args.get(1) {
            Some(p) => std::fs::read_to_string(p).context("read set")?,
            None => LIVE_SET.to_string(),
        };
        let set = serde_json::from_str(&set).context("parse set")?;
        return print("live", &eval::live::run(&set, &live_router()?).await?);
    }
    let [cmd, reference, hypothesis] = args.as_slice() else {
        bail!("usage: eval [live [set.json]] | eval wer <reference.txt> <hypothesis.txt> | eval ask <set.json>");
    };
    if cmd != "wer" {
        bail!("unknown command {cmd}; try `wer`");
    }
    let read = |p: &String| std::fs::read_to_string(p).with_context(|| format!("read {p}"));
    let r = eval::report(&read(reference)?, &read(hypothesis)?);
    println!("wer {:.4}", r.wer);
    println!("cer {:.4}", r.cer);
    println!("wer_no_diacritics {:.4}", r.wer_no_diacritics);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn live_config_defaults_to_openai() {
        let got = live_config(env(&[("RECAPZ_LLM_KEY", "k"), ("RECAPZ_EVAL_MODEL", "m")])).unwrap();
        assert_eq!(
            got,
            ("k".into(), "m".into(), "https://api.openai.com/v1".into())
        );
    }

    #[test]
    fn live_config_takes_a_base_url() {
        let got = live_config(env(&[
            ("RECAPZ_LLM_KEY", "k"),
            ("RECAPZ_EVAL_MODEL", "m"),
            ("RECAPZ_EVAL_BASE_URL", "http://localhost:1/v1"),
        ]))
        .unwrap();
        assert_eq!(got.2, "http://localhost:1/v1");
    }

    #[test]
    fn live_config_names_the_missing_variable() {
        let e = live_config(env(&[("RECAPZ_EVAL_MODEL", "m")]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("RECAPZ_LLM_KEY is not set"), "{e}");
        let e = live_config(env(&[("RECAPZ_LLM_KEY", "k")]))
            .unwrap_err()
            .to_string();
        assert_eq!(e, "RECAPZ_EVAL_MODEL is not set");
    }
}
