//! Cliente CDP mínimo sobre WebSocket.
//!
//! No usamos `chromiumoxide` aquí a propósito: necesitamos control exacto de
//! `Fetch.requestPaused` (binding antes de enviar) y de los eventos de navegación.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    sink: tokio::sync::Mutex<futures_util::stream::SplitSink<Socket, Message>>,
}

/// Conexión CDP con una pestaña.
pub struct Cdp {
    inner: std::sync::Arc<Inner>,
    events: mpsc::UnboundedReceiver<Value>,
    _reader: tokio::task::JoinHandle<()>,
}

impl Cdp {
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (socket, _response) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .with_context(|| format!("timeout conectando a {ws_url}"))?
        .with_context(|| format!("no se pudo conectar a {ws_url}"))?;
        let (sink, mut stream) = socket.split();
        let inner = std::sync::Arc::new(Inner {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            sink: tokio::sync::Mutex::new(sink),
        });
        let (events_tx, events) = mpsc::unbounded_channel();

        let reader_inner = inner.clone();
        let reader = tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let Ok(message) = message else { break };
                let Some(text) = message.to_text().ok() else {
                    continue;
                };
                let Ok(value) = serde_json::from_str::<Value>(text) else {
                    continue;
                };
                match value.get("id").and_then(Value::as_u64) {
                    Some(id) => {
                        let sender = reader_inner.pending.lock().unwrap().remove(&id);
                        if let Some(sender) = sender {
                            let _ = sender.send(value);
                        }
                    }
                    None => {
                        if matches!(
                            value["method"].as_str(),
                            Some(
                                "Fetch.requestPaused"
                                    | "Page.screencastFrame"
                                    | "Page.navigatedWithinDocument"
                                    | "Page.domContentEventFired"
                            )
                        ) {
                            let _ = events_tx.send(value);
                        }
                    }
                }
            }
        });

        Ok(Cdp {
            inner,
            events,
            _reader: reader,
        })
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);

        let payload = json!({ "id": id, "method": method, "params": params }).to_string();
        {
            let mut sink = self.inner.sink.lock().await;
            sink.send(Message::Text(payload.into()))
                .await
                .context("fallo al escribir en CDP")?;
        }

        let response = tokio::time::timeout(Duration::from_secs(15), rx)
            .await
            .with_context(|| format!("timeout esperando respuesta de {method}"))?
            .context("la conexión CDP se cerró")?;

        if let Some(error) = response.get("error") {
            bail!("CDP {method} devolvió error: {error}");
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Espera el siguiente evento. `None` si la conexión terminó.
    pub async fn next_event(&mut self, timeout: Duration) -> Result<Option<Value>> {
        if timeout.is_zero() {
            return Ok(self.events.try_recv().ok());
        }
        match tokio::time::timeout(timeout, self.events.recv()).await {
            Ok(Some(event)) => Ok(Some(event)),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }
}
