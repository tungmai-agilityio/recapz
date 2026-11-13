//! Helpers for the real-browser tests.
use std::{
    io::{Read, Write},
    net::TcpListener,
};

/// Serves `html` for every request on localhost (a secure context, which audio worklets need) and
/// returns its base URL.
pub fn serve(html: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", l.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut c in l.incoming().flatten() {
            let mut buf = [0u8; 2048];
            let _ = c.read(&mut buf);
            let _ = write!(
                c,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
        }
    });
    url
}

static BROWSER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Real-browser tests run one at a time: several Chromium instances at once make the audio timing
/// test miss its tolerance on a busy machine.
pub async fn exclusive() -> tokio::sync::MutexGuard<'static, ()> {
    BROWSER.lock().await
}
