use super::*;
use tokio::net::TcpListener;

type Sock = WebSocketStream<TcpStream>;

/// A sidecar that accepts one connection, reads the join frame, runs `script`, then returns every
/// text frame the driver sent (the join frame first, the leave frame last).
async fn fake<F, Fut>(script: F) -> (String, tokio::task::JoinHandle<Vec<String>>)
where
    F: FnOnce(Sock) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Sock> + Send,
{
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    let h = tokio::spawn(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
        let join = ws.next().await.unwrap().unwrap().into_text().unwrap();
        let mut got = vec![join.to_string()];
        let mut ws = script(ws).await;
        while let Ok(Some(Ok(m))) = timeout(Duration::from_millis(500), ws.next()).await {
            if let Message::Text(t) = m {
                got.push(t.to_string());
            }
        }
        got
    });
    (url, h)
}

fn cfg(url: &str) -> ZoomConfig {
    ZoomConfig {
        sidecar_url: url.into(),
        name: "Recapz notetaker".into(),
        notice: "Recording. Type 'opt out' to stop.".into(),
    }
}

async fn send(ws: &mut Sock, v: Value) {
    ws.send(Message::text(v.to_string())).await.unwrap();
}

/// The next event that is not a clock tick.
async fn next(d: &mut ZoomDriver) -> Result<DriverEvent, DriverError> {
    loop {
        match d.next().await {
            Ok(DriverEvent::Call(Event::Tick { .. })) => continue,
            other => return other,
        }
    }
}

#[tokio::test]
async fn join_frame_then_every_frame_kind_maps_to_its_event_in_order() {
    let (url, h) = fake(|mut ws| async move {
        send(&mut ws, json!({"type": "admitted"})).await;
        send(&mut ws, json!({"type": "participants", "count": 3})).await;
        send(&mut ws, json!({"type": "unknown_future_frame"})).await;
        send(
            &mut ws,
            json!({"type": "speaker", "at_ms": 250, "name": "Ann"}),
        )
        .await;
        ws.send(Message::binary(vec![1, 0, 254, 255]))
            .await
            .unwrap();
        send(
            &mut ws,
            json!({"type": "chat", "text": "opt out", "from_self": false}),
        )
        .await;
        send(
            &mut ws,
            json!({"type": "notice_missing", "step": "chat_off"}),
        )
        .await;
        send(&mut ws, json!({"type": "ended"})).await;
        ws
    })
    .await;
    let mut d = ZoomDriver::new(cfg(&url));
    d.join("https://zoom.us/j/123456789").await.unwrap();
    assert_eq!(next(&mut d).await, Ok(DriverEvent::Call(Event::Admitted)));
    assert!(matches!(
        next(&mut d).await,
        Ok(DriverEvent::Call(Event::Participants { count: 3, .. }))
    ));
    assert_eq!(
        next(&mut d).await,
        Ok(DriverEvent::Speaker(Sample {
            at_ms: 250,
            speaker: "Ann".into()
        }))
    );
    assert_eq!(next(&mut d).await, Ok(DriverEvent::Audio(vec![1, -2])));
    assert_eq!(
        next(&mut d).await,
        Ok(DriverEvent::Call(Event::Chat {
            text: "opt out".into(),
            from_self: false
        }))
    );
    assert_eq!(
        next(&mut d).await,
        Ok(DriverEvent::NoticeMissing {
            step: "chat_off".into()
        })
    );
    assert!(matches!(
        next(&mut d).await,
        Ok(DriverEvent::Call(Event::CallEnded { .. }))
    ));
    d.leave().await;
    d.leave().await; // safe twice
    let got = h.await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&got[0]).unwrap(),
        json!({"type": "join", "url": "https://zoom.us/j/123456789", "name": "Recapz notetaker", "notice": "Recording. Type 'opt out' to stop."})
    );
    assert_eq!(got[1..], [r#"{"type":"leave"}"#]);
}

#[tokio::test]
async fn denied_removed_and_an_error_frame_map_to_the_session_rules() {
    let (url, _h) = fake(|mut ws| async move {
        send(&mut ws, json!({"type": "denied"})).await;
        send(&mut ws, json!({"type": "removed"})).await;
        send(&mut ws, json!({"type": "error", "reason": "bad signature"})).await;
        ws
    })
    .await;
    let mut d = ZoomDriver::new(cfg(&url));
    d.join("u").await.unwrap();
    assert_eq!(next(&mut d).await, Ok(DriverEvent::Call(Event::Denied)));
    assert_eq!(next(&mut d).await, Ok(DriverEvent::Call(Event::Removed)));
    assert_eq!(next(&mut d).await, Err(DriverError::Disconnected));
}

#[tokio::test]
async fn a_sidecar_that_closes_or_never_answers_is_disconnected() {
    let (url, _h) = fake(|mut ws| async move {
        ws.close(None).await.unwrap();
        ws
    })
    .await;
    let mut d = ZoomDriver::new(cfg(&url));
    d.join("u").await.unwrap();
    assert_eq!(next(&mut d).await, Err(DriverError::Disconnected));

    let (url, _h) = fake(|ws| async move {
        tokio::time::sleep(Duration::from_secs(2)).await; // reads the join, then says nothing
        ws
    })
    .await;
    let mut d = ZoomDriver::with_idle(cfg(&url), Duration::from_millis(200));
    d.join("u").await.unwrap();
    let from = std::time::Instant::now();
    assert_eq!(next(&mut d).await, Err(DriverError::Disconnected));
    assert!(from.elapsed() >= Duration::from_millis(150));
}

#[tokio::test]
async fn no_sidecar_listening_fails_the_join() {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    drop(l);
    let mut d = ZoomDriver::new(cfg(&url));
    assert_eq!(d.join("u").await, Err(DriverError::Disconnected));
}
