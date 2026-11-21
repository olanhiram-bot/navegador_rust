//! Permisos por origen: se conceden y se deniegan desde la shell, sin depender
//! de la burbuja de Chromium (que en el display virtual nadie puede pulsar).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

use engine_cdp::Chrome;

fn fixture() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let body = if head.starts_with("GET /favicon.ico") {
                ""
            } else {
                "<body><script>setInterval(()=>{document.body.textContent=Notification.permission},50)</script></body>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

async fn wait_text(chrome: &mut Chrome, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = chrome.read_text().await.unwrap_or_default();
        if text.trim() == wanted {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "el permiso no llegó a {wanted:?}, último {text:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conceder_y_denegar_notificaciones_cambia_lo_que_ve_la_pagina() {
    let base = fixture();
    let mut chrome = Chrome::launch().await.expect("Chromium debe lanzarse");
    chrome.navigate(&format!("{base}/")).await.unwrap();

    chrome
        .set_permissions(&base, &["notifications".into()], &[])
        .await
        .expect("conceder");
    wait_text(&mut chrome, "granted").await;

    chrome
        .set_permissions(&base, &[], &["notifications".into()])
        .await
        .expect("denegar");
    wait_text(&mut chrome, "denied").await;
}
