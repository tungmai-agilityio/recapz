use anyhow::{bail, Context, Result};

/// `cargo run -p eval -- wer <reference.txt> <hypothesis.txt>`
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [cmd, reference, hypothesis] = args.as_slice() else {
        bail!("usage: eval wer <reference.txt> <hypothesis.txt>");
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
