//! Puente entre la shell y Chromium.
//!
//! El hilo del motor es el único dueño del navegador. La UI manda comandos y
//! recibe frames ya decodificados a RGBA, snapshots y el informe de handoff.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
use engine_cdp::Chrome;
use receipt::ReceiptLog;
use serde_json::Value;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// Un frame de la página ya decodificado.
#[derive(Debug)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Tamaño CSS de la página, para mapear el puntero.
    pub css_width: f64,
    pub css_height: f64,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub url: String,
    pub title: String,
    pub generation: u32,
    pub elements: usize,
}

#[derive(Debug)]
pub enum Event {
    Frame(Frame),
    Snapshot(SnapshotInfo),
    Handoff(session::Handoff),
    Error(String),
}

pub enum Command {
    Navigate(String),
    Wheel { x: f64, y: f64, delta_y: f64 },
    Handoff,
    Shutdown,
}

/// Mando de la shell hacia el motor.
pub struct EngineHandle {
    pub commands: UnboundedSender<Command>,
    pub events: Receiver<Event>,
    done: Receiver<()>,
}

impl EngineHandle {
    pub fn navigate(&self, url: impl Into<String>) {
        let _ = self.commands.send(Command::Navigate(url.into()));
    }

    pub fn wheel(&self, x: f64, y: f64, delta_y: f64) {
        let _ = self.commands.send(Command::Wheel { x, y, delta_y });
    }

    pub fn handoff(&self) {
        let _ = self.commands.send(Command::Handoff);
    }

    /// Espera a que el motor cierre Chromium. Acotado para no colgar la salida.
    pub fn shutdown(&self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.done.recv_timeout(Duration::from_secs(5));
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.done.recv_timeout(Duration::from_secs(3));
    }
}

pub fn spawn(initial_url: String, log_path: PathBuf) -> EngineHandle {
    let (commands, command_rx) = unbounded_channel();
    let (events_tx, events) = channel();
    let (done_tx, done) = channel();

    std::thread::spawn(move || {
        if let Err(error) = run(initial_url, log_path, command_rx, events_tx.clone()) {
            let _ = events_tx.send(Event::Error(format!("{error:#}")));
        }
        let _ = done_tx.send(());
    });

    EngineHandle {
        commands,
        events,
        done,
    }
}

fn run(
    initial_url: String,
    log_path: PathBuf,
    mut commands: UnboundedReceiver<Command>,
    events: Sender<Event>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("crear el runtime del motor")?;

    runtime.block_on(async move {
        let mut chrome = Chrome::launch().await.context("lanzar Chromium")?;
        let mut generation = 0u32;

        navigate(&mut chrome, &initial_url, &mut generation, &events).await?;
        chrome
            .start_screencast()
            .await
            .context("iniciar el screencast")?;

        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Shutdown) => {
                        let _ = chrome.stop_screencast().await;
                        return Ok(());
                    }
                    Ok(Command::Navigate(url)) => {
                        navigate(&mut chrome, &url, &mut generation, &events).await?;
                        chrome.start_screencast().await.ok();
                    }
                    Ok(Command::Wheel { x, y, delta_y }) => {
                        chrome.scroll(x, y, delta_y).await?;
                    }
                    Ok(Command::Handoff) => {
                        send_handoff(&mut chrome, generation, &log_path, &events).await?;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }

            if let Some(event) = chrome.next_event(Duration::from_millis(50)).await?
                && event.get("method").and_then(Value::as_str) == Some("Page.screencastFrame")
            {
                handle_frame(&mut chrome, &event, &events).await;
            }
        }
    })
}

async fn navigate(
    chrome: &mut Chrome,
    url: &str,
    generation: &mut u32,
    events: &Sender<Event>,
) -> Result<()> {
    chrome.navigate(url).await?;
    *generation += 1;
    let snapshot = chrome.snapshot(*generation).await?;
    let _ = events.send(Event::Snapshot(SnapshotInfo {
        url: snapshot.url,
        title: snapshot.title,
        generation: *generation,
        elements: snapshot.elements.len(),
    }));
    Ok(())
}

async fn handle_frame(chrome: &mut Chrome, event: &Value, events: &Sender<Event>) {
    let params = &event["params"];
    let Some(session_id) = params.get("sessionId").and_then(Value::as_i64) else {
        return;
    };
    let Some(data) = params.get("data").and_then(Value::as_str) else {
        return;
    };
    let metadata = params.get("metadata").cloned().unwrap_or(Value::Null);
    let css_width = metadata
        .get("deviceWidth")
        .and_then(Value::as_f64)
        .unwrap_or(1024.0);
    let css_height = metadata
        .get("deviceHeight")
        .and_then(Value::as_f64)
        .unwrap_or(768.0);

    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data)
        && let Ok(image) = image::load_from_memory(&bytes)
    {
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let _ = events.send(Event::Frame(Frame {
            width: width as usize,
            height: height as usize,
            rgba: rgba.into_raw(),
            css_width,
            css_height,
        }));
    }

    let _ = chrome.ack_screencast_frame(session_id).await;
}

async fn send_handoff(
    chrome: &mut Chrome,
    generation: u32,
    log_path: &Path,
    events: &Sender<Event>,
) -> Result<()> {
    let snapshot = chrome.snapshot(generation).await?;
    let (receipts, last_receipt_hash) = match ReceiptLog::open(log_path) {
        Ok(log) => (
            log.entries().len(),
            log.last_receipt()
                .map(|receipt| receipt.descriptor.hash_hex.clone()),
        ),
        Err(error) => {
            let _ = events.send(Event::Error(format!("log de recibos: {error}")));
            (0, None)
        }
    };

    let _ = events.send(Event::Handoff(session::Handoff {
        url: snapshot.url,
        title: snapshot.title,
        generation,
        elements: snapshot.elements,
        receipts,
        last_receipt_hash,
        log_path: log_path.to_path_buf(),
    }));
    Ok(())
}
