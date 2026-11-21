//! El puente de la shell contra Chromium real: frames, snapshot, pestañas y
//! actos con tarjeta y recibo.

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use session::ActOutcome;
use shell::engine::{EngineHandle, Event, spawn};

fn wait_for(
    handle: &EngineHandle,
    seconds: u64,
    mut predicate: impl FnMut(&Event) -> bool,
) -> Event {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "no llegó el evento esperado");
        match handle
            .events
            .recv_timeout(left.min(Duration::from_millis(500)))
        {
            Ok(Event::Error(error)) => panic!("error del motor: {error}"),
            Ok(event) => {
                if predicate(&event) {
                    return event;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("el motor cerró el canal"),
        }
    }
}

fn first_snapshot(handle: &EngineHandle) -> (shell::engine::TabId, Vec<engine_cdp::ElementRef>) {
    match wait_for(handle, 20, |event| matches!(event, Event::Snapshot { .. })) {
        Event::Snapshot {
            tab,
            info,
            elements,
            ..
        } => {
            assert!(info.elements > 0);
            assert_eq!(info.elements, elements.len());
            (tab, elements)
        }
        _ => unreachable!(),
    }
}

#[test]
fn la_pagina_se_ve_y_llega_el_snapshot() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );

    let (tab, _) = first_snapshot(&handle);
    let frame = wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Frame { tab: t, .. } if *t == tab),
    );
    if let Event::Frame { frame, .. } = frame {
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(frame.rgba.len(), frame.width * frame.height * 4);
        assert!(frame.css_width > 0.0 && frame.css_height > 0.0);
    }

    handle.shutdown();
}

#[test]
fn click_sobre_un_boton_pasa_por_tarjeta_y_recibo() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );

    let (_, elements) = first_snapshot(&handle);
    let button = elements
        .iter()
        .find(|element| element.role == "button" && element.name.contains("Pagar"))
        .expect("el fixture tiene un botón «Pagar»");

    handle.click(button.x, button.y);

    let (hash, card) = match wait_for(&handle, 15, |event| {
        matches!(
            event,
            Event::Act {
                outcome: ActOutcome::ApprovalRequired { .. },
                ..
            }
        )
    }) {
        Event::Act {
            outcome: ActOutcome::ApprovalRequired { hash, card, .. },
            ..
        } => (hash, card),
        _ => unreachable!(),
    };
    assert!(
        card.rows.iter().any(|row| row.value.contains("24.90")),
        "la tarjeta debe mostrar el importe real: {:?}",
        card.rows
    );

    handle.approve(hash);

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut seq = None;
    let mut receipts = None;
    while seq.is_none() || receipts.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "faltó el acto ejecutado o el recibo");
        match handle.events.recv_timeout(left) {
            Ok(Event::Error(error)) => panic!("error del motor: {error}"),
            Ok(Event::Act {
                outcome: ActOutcome::Executed { seq: n, .. },
                ..
            }) => seq = Some(n),
            Ok(Event::Receipts {
                count: 1,
                last,
                log,
                ..
            }) => receipts = Some((last, log)),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("el motor cerró el canal"),
        }
    }

    let (last, log) = receipts.expect("recibo emitido");
    assert!(last.is_some(), "el recibo debe llevar hash");
    let logged = receipt::ReceiptLog::open(&log).expect("abrir el log de la pestaña");
    assert_eq!(logged.entries().len(), 1);
    logged
        .verify()
        .expect("la cadena del recibo debe verificar");
    assert_eq!(seq, Some(0), "el primer acto de la pestaña es el recibo 0");

    handle.shutdown();
}

#[test]
fn atras_y_adelante_recorren_el_historial() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );
    let (tab, _) = first_snapshot(&handle);

    handle.navigate(server.page_url("settings.html"));
    match wait_for(&handle, 20, |event| {
        matches!(event, Event::Snapshot { tab: t, info, .. }
            if *t == tab && info.url.ends_with("settings.html"))
    }) {
        Event::Snapshot { info, .. } => assert!(info.url.ends_with("settings.html")),
        _ => unreachable!(),
    }

    handle.back();
    match wait_for(&handle, 20, |event| {
        matches!(event, Event::Snapshot { tab: t, info, .. }
            if *t == tab && info.url.ends_with("shop.html"))
    }) {
        Event::Snapshot { info, .. } => assert!(info.url.ends_with("shop.html")),
        _ => unreachable!(),
    }

    handle.forward();
    match wait_for(&handle, 20, |event| {
        matches!(event, Event::Snapshot { tab: t, info, .. }
            if *t == tab && info.url.ends_with("settings.html"))
    }) {
        Event::Snapshot { info, .. } => assert!(info.url.ends_with("settings.html")),
        _ => unreachable!(),
    }

    handle.shutdown();
}

#[test]
fn las_pestanas_comparten_ventana_y_se_pueden_cerrar() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );

    let (first, _) = first_snapshot(&handle);
    handle.new_tab(Some(server.page_url("settings.html")));

    let second = match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Selected(Some(id)) if *id != first),
    ) {
        Event::Selected(Some(id)) => id,
        _ => unreachable!(),
    };
    match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Snapshot { tab, .. } if *tab == second),
    ) {
        Event::Snapshot { info, .. } => {
            assert!(info.url.ends_with("settings.html"), "url: {}", info.url);
        }
        _ => unreachable!(),
    }

    handle.select_tab(first);
    match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Snapshot { tab, .. } if *tab == first),
    ) {
        Event::Snapshot { info, .. } => {
            assert!(info.url.ends_with("shop.html"), "url: {}", info.url);
        }
        _ => unreachable!(),
    }

    handle.close_tab(second);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::TabClosed(id) if *id == second),
    );

    handle.close_tab(first);
    wait_for(&handle, 10, |event| matches!(event, Event::Selected(None)));
    handle.navigate(server.page_url("settings.html"));
    wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Snapshot { tab, info, .. } if *tab != first && info.url.ends_with("settings.html")),
    );

    handle.shutdown();
}

#[test]
fn seleccion_copia_y_enlace_con_pantalla_de_alta_densidad() {
    let server = navegador::FixtureServer::start();
    let destination = server.page_url("shop.html");
    let url = format!(
        "data:text/html,<body style='margin:0'><p style='margin:0;font:20px monospace'>Seleccionable</p><a href='{destination}'>Abrir</a><div style='position:absolute;left:300px;top:100px;width:20px;height:20px;background:red'></div>"
    );
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn(url, dir.path().join("log"));
    let (tab, _) = first_snapshot(&handle);
    handle.viewport(1000, 600, 2.0);
    let frame = wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Frame {frame,..} if frame.width == 2000 && frame.height == 1200),
    );
    if let Event::Frame { frame, .. } = frame {
        let offset = 4 * (210 * frame.width + 610);
        assert!(
            frame.rgba[offset] > 220 && frame.rgba[offset + 1] < 40,
            "posición física incorrecta"
        );
        assert_eq!((frame.css_width, frame.css_height), (1000.0, 600.0));
    }
    handle.select_text(tab, [0.0, 10.0], [250.0, 10.0]);
    handle.copy_text(tab);
    wait_for(
        &handle,
        5,
        |event| matches!(event, Event::CopyText(_, text) if text == "Seleccionable"),
    );
    handle.click(15.0, 32.0);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Snapshot {info,..} if info.url == destination),
    );
    handle.shutdown();
}

#[test]
fn una_redireccion_tardia_actualiza_la_barra() {
    let server = navegador::FixtureServer::start();
    let destination = server.page_url("shop.html");
    let initial = format!(
        "data:text/html,<a href='{destination}'>Abrir</a><script>setTimeout(()=>location.replace('{destination}'),1800)</script>"
    );
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn(initial, dir.path().join("log"));
    let (tab, _) = first_snapshot(&handle);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Snapshot {tab: id, info, ..} if *id == tab && info.url == destination),
    );
    handle.shutdown();
}

#[test]
fn se_abre_un_enlace_que_aparece_despues_del_snapshot() {
    let server = navegador::FixtureServer::start();
    let destination = server.page_url("shop.html");
    let initial = format!(
        "data:text/html,<body style='margin:0'><button>Inicio</button><script>setTimeout(()=>{{const a=document.createElement('a');a.href='{destination}';a.innerText='Video';a.style='position:absolute;left:0;top:50px';document.body.append(a)}},1000)</script>"
    );
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn(initial, dir.path().join("log"));
    first_snapshot(&handle);
    std::thread::sleep(Duration::from_millis(1300));
    handle.click(9.0, 58.0);
    wait_for(
        &handle,
        10,
        |event| matches!(event,Event::Snapshot {info,..} if info.url == destination),
    );
    handle.shutdown();
}

#[test]
fn una_imagen_lenta_no_retrasa_la_navegacion() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", server.local_addr().unwrap());
    let (finish_image, wait_for_snapshot) = std::sync::mpsc::channel();
    let server_thread = std::thread::spawn(move || {
        for stream in server.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0; 4096];
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let Ok(bytes) = stream.read(&mut request) else {
                continue;
            };
            if bytes == 0 {
                continue;
            }
            let path = String::from_utf8_lossy(&request[..bytes]);
            if path.starts_with("GET /slow.png ") {
                let _ = wait_for_snapshot.recv_timeout(Duration::from_secs(12));
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            let html =
                "<title>Página lista</title><img src='/slow.png'><a href='#listo'>Continuar</a>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
                html.len()
            );
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let handle = spawn("about:blank".into(), dir.path().join("log"));
    wait_for(&handle, 10, |event| matches!(event, Event::Snapshot { .. }));
    handle.navigate(url);
    wait_for(
        &handle,
        5,
        |event| matches!(event, Event::Snapshot { info, .. } if info.title == "Página lista"),
    );
    finish_image.send(()).unwrap();
    server_thread.join().unwrap();
    handle.shutdown();
}

#[test]
fn restaura_varias_pestanas_y_reutiliza_el_perfil_al_reabrir() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("perfil");
    let open = |urls: Vec<String>| {
        shell::engine::spawn_with_options(
            shell::engine::Options {
                initial_urls: urls,
                log_path: dir.path().join("recibos.jsonl"),
                profile: Some(profile.clone()),
            },
            std::sync::Arc::new(|| {}),
        )
    };

    let handle = open(vec![
        server.page_url("shop.html"),
        server.page_url("settings.html"),
    ]);
    let mut opened = Vec::new();
    while opened.len() < 2 {
        if let Event::TabOpened(info) =
            wait_for(&handle, 30, |event| matches!(event, Event::TabOpened(_)))
        {
            opened.push(info.url);
        }
    }
    assert_eq!(
        opened.len(),
        2,
        "deben abrirse las dos pestañas restauradas"
    );
    handle.shutdown();
    assert!(
        profile.is_dir(),
        "el perfil persistente debe sobrevivir al cierre: {profile:?}"
    );

    // El candado se suelta al cerrar: reabrir el mismo perfil debe funcionar.
    let handle = open(vec![server.page_url("shop.html")]);
    first_snapshot(&handle);
    handle.shutdown();
}

#[test]
fn enter_en_un_campo_pasa_por_la_tarjeta_y_no_escapa_solo() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    // El servidor registra lo que llega: nada debe salir antes de aprobar.
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let url = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = vec![0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]).to_string();
            let (start, body) = head.split_once("\r\n\r\n").unwrap_or((&head, ""));
            let mut request = start.lines().next().unwrap_or("").to_string();
            if let Some(length) = start.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            }) && body.len() < length
            {
                let mut rest = vec![0u8; length - body.len()];
                let extra = stream.read(&mut rest).unwrap_or(0);
                request.push_str(&String::from_utf8_lossy(&rest[..extra]));
            }
            if start.starts_with("POST") {
                record.lock().unwrap().push(format!("{request} {body}"));
            }
            let page = if start.starts_with("POST") {
                "<body>gracias</body>"
            } else {
                "<body><form method=POST action=/enviar><input name=q><button>Enviar</button></form></body>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                page.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(url, dir.path().join("recibos.jsonl"));
    let (_, elements) = first_snapshot(&handle);
    let field = elements
        .iter()
        .find(|element| element.role == "textbox")
        .expect("el formulario tiene un campo de texto");
    handle.click(field.x, field.y);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Notice(notice) if notice.contains("enfocado")),
    );
    handle.insert_text("hola");
    std::thread::sleep(Duration::from_millis(300));

    handle.key("Enter", "Enter", 13, 0);
    let hash = match wait_for(&handle, 15, |event| {
        matches!(
            event,
            Event::Act {
                outcome: ActOutcome::ApprovalRequired { .. },
                ..
            }
        )
    }) {
        Event::Act {
            outcome: ActOutcome::ApprovalRequired { hash, card, .. },
            ..
        } => {
            assert!(
                card.rows.iter().any(|row| row.value.contains("hola")),
                "la tarjeta debe mostrar lo que se va a enviar: {:?}",
                card.rows
            );
            hash
        }
        _ => unreachable!(),
    };
    assert!(
        seen.lock().unwrap().is_empty(),
        "no debe escapar ninguna petición antes de aprobar: {:?}",
        seen.lock().unwrap()
    );

    handle.approve(hash);
    let deadline = Instant::now() + Duration::from_secs(15);
    let (mut executed, mut receipts) = (false, false);
    while !executed || !receipts {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "faltó el acto ejecutado o el recibo");
        match handle
            .events
            .recv_timeout(left.min(Duration::from_millis(500)))
        {
            Ok(Event::Error(error)) => panic!("error del motor: {error}"),
            Ok(Event::Act {
                outcome: ActOutcome::Executed { .. },
                ..
            }) => executed = true,
            Ok(Event::Receipts { count: 1, .. }) => receipts = true,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("el motor cerró el canal"),
        }
    }
    let sent = seen.lock().unwrap().clone();
    assert_eq!(sent.len(), 1, "debe llegar exactamente un envío: {sent:?}");
    assert!(sent[0].contains("q=hola"), "envío inesperado: {sent:?}");
    handle.shutdown();
}
