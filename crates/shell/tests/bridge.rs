//! El puente de la shell contra Chromium real: frames y snapshot.

use std::time::{Duration, Instant};

use shell::engine::{Event, spawn};

#[test]
fn la_shell_recibe_frames_y_snapshot() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );

    let mut saw_snapshot = false;
    let mut saw_frame = false;
    let deadline = Instant::now() + Duration::from_secs(20);

    while Instant::now() < deadline && !(saw_snapshot && saw_frame) {
        match handle.events.recv_timeout(Duration::from_millis(500)) {
            Ok(Event::Snapshot(info)) => {
                assert!(info.url.ends_with("shop.html"), "url: {}", info.url);
                assert!(info.elements > 0, "la página debe tener elementos");
                saw_snapshot = true;
            }
            Ok(Event::Frame(frame)) => {
                assert!(frame.width > 0 && frame.height > 0);
                assert_eq!(frame.rgba.len(), frame.width * frame.height * 4);
                assert!(frame.css_width > 0.0 && frame.css_height > 0.0);
                saw_frame = true;
            }
            Ok(Event::Error(error)) => panic!("error del motor: {error}"),
            Ok(Event::Handoff(_)) => {}
            Err(_) => {}
        }
    }

    assert!(
        saw_snapshot && saw_frame,
        "snapshot={saw_snapshot} frame={saw_frame}"
    );

    handle.shutdown();
}
