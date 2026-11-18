use recapz_speech::deepgram::DeepgramTranscriber;

/// The server builds its transcriber inside `#[tokio::main]`; reqwest's blocking client panics
/// when built on a thread that runs async code.
#[tokio::test]
async fn the_transcriber_can_be_built_inside_an_async_runtime() {
    let t = DeepgramTranscriber::new("http://127.0.0.1:1", "k").expect("built inside a runtime");
    // Dropped on a plain thread, as the job runner does.
    std::thread::spawn(move || drop(t)).join().unwrap();
}
