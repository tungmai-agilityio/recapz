use anyhow::{bail, Context, Result};
use eval::ask::{AskReport, PASS_RATE};
use recapz_llm::{anthropic::Anthropic, Price, Route, Router, Task};
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

/// Anthropic, with the key and model taken from the environment (never printed or stored).
fn live_router() -> Result<Router> {
    let key = std::env::var("ANTHROPIC_API_KEY").context(
        "ANTHROPIC_API_KEY is not set; the live answer eval calls a model (synthetic text only)",
    )?;
    let model = std::env::var("RECAPZ_EVAL_MODEL").context("RECAPZ_EVAL_MODEL is not set")?;
    Ok(Router::new()
        .provider("anthropic", Arc::new(Anthropic::new(key)))
        .route(
            Task::LiveAnswer,
            Route {
                provider: "anthropic".into(),
                model,
                max_tokens: 1024,
                price: Price::default(),
            },
        ))
}

/// `cargo run -p eval` or `-- live [set.json]`: score live answers with a model (needs ANTHROPIC_API_KEY, RECAPZ_EVAL_MODEL)
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
