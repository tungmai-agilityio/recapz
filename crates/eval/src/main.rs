use anyhow::{bail, Context, Result};

/// `cargo run -p eval -- wer <reference.txt> <hypothesis.txt>`
/// `cargo run -p eval -- ask <set.json>` (exits 1 below the 80% pass rate)
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [cmd, set] = args.as_slice() {
        if cmd == "ask" {
            let set = serde_json::from_str(&std::fs::read_to_string(set).context("read set")?)
                .context("parse set")?;
            let r = eval::ask::run(&set)?;
            println!("ask_total {}", r.total);
            println!("ask_correct {}", r.correct);
            println!("ask_accuracy {:.4}", r.accuracy());
            for m in &r.misses {
                println!("miss: {m}");
            }
            if r.accuracy() < eval::ask::PASS_RATE {
                std::process::exit(1);
            }
            return Ok(());
        }
    }
    let [cmd, reference, hypothesis] = args.as_slice() else {
        bail!("usage: eval wer <reference.txt> <hypothesis.txt> | eval ask <set.json>");
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
