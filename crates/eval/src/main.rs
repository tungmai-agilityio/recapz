use anyhow::{bail, Context, Result};
use eval::ask::{AskReport, Sweep, PASS_RATE};
use recapz_core::embed::HashEmbedder;
use recapz_llm::{openai_compat::OpenAiCompat, Price, Route, Router, Task};
use recapz_server::config::{Config, Llm};
use serde_json::Value;
use std::sync::Arc;

const LIVE_SET: &str = include_str!("../fixtures/live-answer-set.json");
const REPORT_SET: &str = include_str!("../fixtures/report-set.json");

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
/// One route shared by every task, as the server does.
fn live_router() -> Result<Router> {
    let (key, model, base) = live_config(|k| std::env::var(k).ok())?;
    let route = |_| Route {
        provider: "openai".into(),
        model: model.clone(),
        // Reasoning models spend max_tokens on thinking before the reply.
        max_tokens: 8192,
        price: Price::default(),
    };
    Ok(Router::new()
        .provider("openai", Arc::new(OpenAiCompat::new(base, key)))
        .route(Task::LiveAnswer, route(()))
        .route(Task::Summary, route(()))
        .route(Task::Decisions, route(()))
        .route(Task::Chapters, route(()))
        .route(Task::ActionItems, route(())))
}

fn sweep_lines(name: &str, s: &Sweep) -> Vec<String> {
    let mut out = vec![format!("ask_embedder {name}")];
    for (t, r) in &s.rates {
        out.push(format!(
            "ask_threshold {t:.2} correct {}/{} rate {:.4}",
            r.correct,
            r.total,
            r.accuracy()
        ));
    }
    out.push(format!("ask_current_threshold {:.2}", s.current.0));
    out
}

/// Embeds with the `[llm]` settings of the server's config. Embedding blocks, so it runs off the async thread.
async fn hosted_sweep(set: Value, llm: Llm) -> Result<Sweep> {
    let e = recapz_server::embedder::from_config("hosted", &llm)?;
    tokio::task::spawn_blocking(move || eval::ask::sweep(&set, e.as_ref())).await?
}

async fn ask(args: &[String]) -> Result<()> {
    let (path, name) = match args {
        [p] => (p, "hash"),
        [p, f, n] if f == "--embedder" => (p, n.as_str()),
        _ => bail!("usage: eval ask <set.json> [--embedder hash|hosted]"),
    };
    let set: Value = serde_json::from_str(&std::fs::read_to_string(path).context("read set")?)
        .context("parse set")?;
    let sweep = match name {
        "hash" => eval::ask::sweep(&set, &HashEmbedder)?,
        "hosted" => {
            let cfg = Config::load(
                &std::env::var("RECAPZ_CONFIG").unwrap_or_else(|_| "config.toml".into()),
            )?;
            hosted_sweep(set, cfg.llm).await?
        }
        other => bail!("unknown embedder {other}; try hash or hosted"),
    };
    for l in sweep_lines(name, &sweep) {
        println!("{l}");
    }
    print("ask", &sweep.current.1)
}

/// `cargo run -p eval` or `-- live [set.json]`: score live answers with a model (needs RECAPZ_LLM_KEY, RECAPZ_EVAL_MODEL; RECAPZ_EVAL_BASE_URL is optional)
/// `cargo run -p eval -- report [set.json]`: score the report prompts (same variables as `live`; exits 1 below 80%)
/// `cargo run -p eval -- wer <reference.txt> <hypothesis.txt>`
/// `cargo run -p eval -- ask <set.json> [--embedder hash|hosted]` (retrieval only, exits 1 below the 80% pass rate at the embedder's threshold;
/// `hosted` reads the server's config file, `RECAPZ_CONFIG` or `config.toml`, and sends the set's text to its embeddings endpoint)
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("ask") {
        return ask(&args[1..]).await;
    }
    if args.first().map(String::as_str) == Some("report") && args.len() <= 2 {
        let set = match args.get(1) {
            Some(p) => std::fs::read_to_string(p).context("read set")?,
            None => REPORT_SET.to_string(),
        };
        let set = serde_json::from_str(&set).context("parse set")?;
        return print("report", &eval::report::run(&set, &live_router()?).await?);
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
        bail!("usage: eval [live [set.json]] | eval report [set.json] | eval wer <reference.txt> <hypothesis.txt> | eval ask <set.json>");
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

    #[tokio::test]
    async fn the_hosted_path_embeds_through_the_configured_endpoint_and_prints_no_secret() {
        use std::sync::{Arc, Mutex};
        let seen: Arc<Mutex<Vec<Value>>> = Default::default();
        let app = axum::Router::new()
            .route(
                "/v1/embeddings",
                axum::routing::post(
                    |axum::extract::State(s): axum::extract::State<Arc<Mutex<Vec<Value>>>>,
                     axum::Json(b): axum::Json<Value>| async move {
                        s.lock().unwrap().push(b);
                        axum::Json(serde_json::json!({ "data": [{ "embedding": [1.0, 0.0] }] }))
                    },
                ),
            )
            .with_state(seen.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let llm = Llm {
            base_url: base,
            api_key: "sk-secret".into(),
            embedding_model: "emb-1".into(),
            ..Llm::default()
        };
        let set: Value = serde_json::from_str(include_str!("../fixtures/ask-sample.json")).unwrap();
        let s = hosted_sweep(set, llm).await.unwrap();
        // Every text has the same vector, so every threshold up to 0.6 keeps every chunk.
        assert_eq!(s.rates.len(), 13);
        assert!(s
            .rates
            .iter()
            .all(|(_, r)| r.total == 12 && r.correct == s.rates[0].1.correct));
        let calls = seen.lock().unwrap();
        assert!(!calls.is_empty() && calls.iter().all(|c| c["model"] == "emb-1"));
        let queries: std::collections::HashSet<_> =
            calls.iter().map(|c| c["input"].to_string()).collect();
        assert_eq!(
            queries.len(),
            calls.len(),
            "each text is embedded once for the whole sweep"
        );
        let out = sweep_lines("hosted", &s).join("\n");
        assert!(!out.contains("sk-secret") && !out.contains("127.0.0.1"));
        assert!(out.starts_with("ask_embedder hosted\nask_threshold 0.00 correct "));
        assert!(out.ends_with("ask_current_threshold 0.20"));
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
