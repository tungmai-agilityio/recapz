//! Joining a Zoom call through the Zoom sidecar, a separate process that runs the Meeting SDK (it must
//! not link into the server). `ZoomDriver` is the `CallDriver` for it and talks to the sidecar over a
//! local WebSocket, the same frame shapes the browser recording socket uses.
//!
//! Protocol. The driver connects, then:
//! - driver to sidecar, text JSON: `{"type":"join","url":..,"name":..,"notice":..}` once, and
//!   `{"type":"leave"}` at the end. The Zoom credentials are the sidecar's own environment; they never
//!   cross this socket and are never logged.
//! - sidecar to driver, text JSON: `{"type":"admitted"}`, `{"type":"denied"}`, `{"type":"removed"}`,
//!   `{"type":"ended"}`, `{"type":"participants","count":n}`, `{"type":"chat","text":..,"from_self":bool}`,
//!   `{"type":"speaker","at_ms":n,"name":..}` (who is speaking now, by participant name, about every
//!   250 ms), `{"type":"notice_missing","step":..}` and `{"type":"error","reason":..}`.
//! - sidecar to driver, binary: 16 kHz mono 16-bit little-endian samples, the participants' own streams
//!   already mixed. Sent only after the chat notice was posted or reported missing.
//!
//! The sidecar is silent for no longer than `idle`; a WebSocket ping counts. Longer than that, or a
//! closed socket, ends the call as `Disconnected`. Frames are never logged: they hold names and chat.
use crate::{
    driver::{CallDriver, DriverError, DriverEvent},
    session::Event,
    speakers::Sample,
};
use futures_util::{SinkExt, StreamExt};
use recapz_core::db::now_ms;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{
    net::TcpStream,
    time::{interval, timeout, Instant, Interval, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

/// How long the sidecar may say nothing at all before the call counts as dropped.
pub const IDLE: Duration = Duration::from_secs(30);
const TICK: Duration = Duration::from_secs(1);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct ZoomConfig {
    /// `ws://127.0.0.1:<port>` of the sidecar.
    pub sidecar_url: String,
    /// The name the notetaker joins as.
    pub name: String,
    /// The chat notice the sidecar posts after admission.
    pub notice: String,
}

pub struct ZoomDriver {
    cfg: ZoomConfig,
    idle: Duration,
    ws: Option<Ws>,
    last_heard: Instant,
    tick: Interval,
}

impl ZoomDriver {
    pub fn new(cfg: ZoomConfig) -> Self {
        Self::with_idle(cfg, IDLE)
    }

    fn with_idle(cfg: ZoomConfig, idle: Duration) -> Self {
        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            cfg,
            idle,
            ws: None,
            last_heard: Instant::now(),
            tick,
        }
    }
}

/// One text frame as a driver event; `None` for a frame that carries nothing for the call.
fn parse(text: &str) -> Result<Option<DriverEvent>, DriverError> {
    let v: Value = serde_json::from_str(text).map_err(|_| DriverError::Disconnected)?;
    let call = |e| Ok(Some(DriverEvent::Call(e)));
    match v["type"].as_str() {
        Some("admitted") => call(Event::Admitted),
        Some("denied") => call(Event::Denied),
        Some("removed") => call(Event::Removed),
        Some("ended") => call(Event::CallEnded { at_ms: now_ms() }),
        Some("participants") => call(Event::Participants {
            at_ms: now_ms(),
            count: v["count"].as_u64().unwrap_or(0) as u32,
        }),
        Some("chat") => call(Event::Chat {
            text: v["text"].as_str().unwrap_or_default().to_string(),
            from_self: v["from_self"].as_bool().unwrap_or(false),
        }),
        Some("speaker") => Ok(v["name"].as_str().map(|n| {
            DriverEvent::Speaker(Sample {
                at_ms: v["at_ms"].as_i64().unwrap_or(0),
                speaker: n.to_string(),
            })
        })),
        Some("notice_missing") => Ok(Some(DriverEvent::NoticeMissing {
            step: v["step"].as_str().unwrap_or("chat").to_string(),
        })),
        // The sidecar could not join or lost the call; its reason is not shown to members.
        Some("error") => Err(DriverError::Disconnected),
        _ => Ok(None), // a newer sidecar's frame: ignored
    }
}

impl CallDriver for ZoomDriver {
    async fn join(&mut self, url: &str) -> Result<(), DriverError> {
        let (mut ws, _) = timeout(self.idle, connect_async(self.cfg.sidecar_url.as_str()))
            .await
            .map_err(|_| DriverError::Disconnected)?
            .map_err(|_| DriverError::Disconnected)?;
        let join =
            json!({"type": "join", "url": url, "name": self.cfg.name, "notice": self.cfg.notice});
        ws.send(Message::text(join.to_string()))
            .await
            .map_err(|_| DriverError::Disconnected)?;
        self.ws = Some(ws);
        self.last_heard = Instant::now();
        Ok(())
    }

    async fn next(&mut self) -> Result<DriverEvent, DriverError> {
        let ws = self.ws.as_mut().ok_or(DriverError::Disconnected)?;
        loop {
            tokio::select! {
                biased;
                m = ws.next() => {
                    self.last_heard = Instant::now();
                    match m {
                        Some(Ok(Message::Text(t))) => {
                            if let Some(e) = parse(t.as_str())? {
                                return Ok(e);
                            }
                        }
                        Some(Ok(Message::Binary(b))) => {
                            let samples = b.as_chunks::<2>().0.iter().map(|p| i16::from_le_bytes(*p)).collect();
                            return Ok(DriverEvent::Audio(samples));
                        }
                        Some(Ok(Message::Close(_)) | Err(_)) | None => return Err(DriverError::Disconnected),
                        Some(Ok(_)) => {} // ping, pong: the sidecar is alive
                    }
                }
                _ = tokio::time::sleep_until(self.last_heard + self.idle) => return Err(DriverError::Disconnected),
                _ = self.tick.tick() => return Ok(DriverEvent::Call(Event::Tick { at_ms: now_ms() })),
            }
        }
    }

    async fn leave(&mut self) {
        if let Some(mut ws) = self.ws.take() {
            let _ = ws
                .send(Message::text(json!({"type": "leave"}).to_string()))
                .await;
            let _ = ws.close(None).await;
        }
    }
}

#[cfg(test)]
mod tests;
