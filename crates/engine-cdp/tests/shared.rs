//! Un navegador por sesión: las pestañas comparten cookies y el perfil puede
//! ser persistente entre arranques.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;

use engine_cdp::{Browser, Chrome, Profile};

/// Servidor mínimo: `/` imprime las cookies; `?set=1` deja una antes.
fn fixture() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (body, content_type) = if path == "/favicon.ico" {
                (String::new(), "image/x-icon")
            } else if path.contains("set=1") {
                (
                    "<body><script>document.cookie='sesion=demo; path=/; max-age=3600';document.body.append(document.cookie)</script></body>"
                        .to_string(),
                    "text/html",
                )
            } else {
                (
                    "<body><script>document.body.append(document.cookie)</script></body>"
                        .to_string(),
                    "text/html",
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dos_pestanas_comparten_cookies() {
    let base = fixture();
    let browser = Arc::new(
        Browser::launch(Profile::ephemeral().unwrap(), false)
            .await
            .expect("Chromium debe lanzarse"),
    );

    let mut first = Chrome::attach(browser.clone()).await.unwrap();
    first.navigate(&format!("{base}/?set=1")).await.unwrap();

    let mut second = Chrome::attach(browser.clone()).await.unwrap();
    second.navigate(&format!("{base}/")).await.unwrap();
    let text = second.read_text().await.unwrap();
    assert!(
        text.contains("sesion=demo"),
        "la segunda pestaña debe ver la cookie de la primera: {text:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn el_perfil_persistente_conserva_las_cookies_tras_reiniciar() {
    let base = fixture();
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("perfil");

    let browser = Arc::new(
        Browser::launch(Profile::persistent(&path), false)
            .await
            .expect("Chromium debe lanzarse"),
    );
    let mut first = Chrome::attach(browser.clone()).await.unwrap();
    first.navigate(&format!("{base}/?set=1")).await.unwrap();
    // Barrera: esperar a que el servicio de red tenga la cookie. Chromium
    // escribe las cookies de forma diferida y un cierre inmediato puede
    // adelantarse al volcado.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if first
            .has_cookie(&format!("{base}/"), "sesion")
            .await
            .unwrap()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "la cookie nunca llegó al servicio de red"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    drop(first);
    // El cierre por SIGTERM (Browser::drop) es el que vacía el almacén de
    // cookies; Browser.close sale demasiado rápido y a veces no lo hace.
    drop(browser);

    assert!(path.is_dir(), "el perfil persistente no debe borrarse");

    let browser = Arc::new(
        Browser::launch(Profile::persistent(&path), false)
            .await
            .expect("el perfil debe reabrirse"),
    );
    let mut second = Chrome::attach(browser.clone()).await.unwrap();
    second.navigate(&format!("{base}/")).await.unwrap();
    // El almacén de cookies se carga del disco en paralelo al arranque: una
    // primera visita muy temprana puede no verla todavía.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let text = second.read_text().await.unwrap();
        if text.contains("sesion=demo") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "la cookie debe sobrevivir al reinicio: {text:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn un_perfil_en_uso_avisa_en_lugar_de_mezclar_sesiones() {
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join("perfil");
    let _first = Browser::launch(Profile::persistent(&path), false)
        .await
        .expect("el primer navegador debe arrancar");

    let error = Browser::launch(Profile::persistent(&path), false)
        .await
        .err()
        .expect("el segundo navegador no debe usar el mismo perfil");
    let message = format!("{error:#}");
    assert!(
        message.contains("está en uso"),
        "el error debe explicar el conflicto de perfil: {message}"
    );
}
