//! El puente de la shell contra Chromium real: frames, snapshot, pestañas y
//! actos con tarjeta y recibo.

use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use session::ActOutcome;
use shell::engine::{EngineHandle, Event, spawn};

static TEST_SLOTS: std::sync::Mutex<usize> = std::sync::Mutex::new(0);
static TEST_SLOT_CV: std::sync::Condvar = std::sync::Condvar::new();

fn slot() -> Slot {
    const MAX: usize = 2;
    let mut in_use = TEST_SLOTS.lock().unwrap();
    while *in_use >= MAX {
        in_use = TEST_SLOT_CV.wait(in_use).unwrap();
    }
    *in_use += 1;
    Slot
}

struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        let mut in_use = TEST_SLOTS.lock().unwrap();
        *in_use -= 1;
        TEST_SLOT_CV.notify_one();
    }
}

fn wait_for(
    handle: &EngineHandle,
    seconds: u64,
    mut predicate: impl FnMut(&Event) -> bool,
) -> Event {
    // Suelo generoso: en una máquina cargada el arranque de Chromium y Xvfb
    // puede tardar bastante más que en una sesión vacía.
    let deadline = Instant::now() + Duration::from_secs(seconds.max(30));
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "no llegó el evento esperado");
        match handle
            .events
            .recv_timeout(left.min(Duration::from_millis(500)))
        {
            Ok(Event::Error(error)) => panic!("error del motor: {error}"),
            Ok(event) => {
                if std::env::var_os("NAVEGADOR_TEST_DEBUG").is_some()
                    && !matches!(event, Event::Frame { .. })
                {
                    eprintln!("evento: {event:?}");
                }
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
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
    let _slot = slot();
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("perfil");
    let open = |urls: Vec<String>| {
        shell::engine::spawn_with_options(
            shell::engine::Options {
                initial_urls: urls,
                log_path: dir.path().join("recibos.jsonl"),
                profile: Some(profile.clone()),
                downloads: None,
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
    let _slot = slot();
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
    let deadline = Instant::now() + Duration::from_secs(30);
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

#[test]
fn un_perfil_ocupado_sigue_navegando_con_un_perfil_temporal() {
    let _slot = slot();
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let profile = dir.path().join("perfil");
    let open = |url: String| {
        shell::engine::spawn_with_options(
            shell::engine::Options {
                initial_urls: vec![url],
                log_path: dir.path().join("recibos.jsonl"),
                profile: Some(profile.clone()),
                downloads: None,
            },
            std::sync::Arc::new(|| {}),
        )
    };

    let first = open(server.page_url("shop.html"));
    first_snapshot(&first);

    // El perfil está tomado por la primera ventana: la segunda debe avisar y
    // seguir funcionando con un perfil temporal, no quedarse vacía.
    let second = open(server.page_url("settings.html"));
    wait_for(
        &second,
        30,
        |event| matches!(event, Event::Notice(notice) if notice.contains("perfil temporal")),
    );
    first_snapshot(&second);
    second.shutdown();
    first.shutdown();
}

#[test]
fn buscar_en_la_pagina_cuenta_y_recorre_las_coincidencias() {
    let _slot = slot();
    let dir = tempfile::tempdir().expect("tempdir");
    let url = "data:text/html,<body><p>hola mundo</p><p>hola otra vez</p><p>adios</p></body>";
    let handle = spawn(url.to_string(), dir.path().join("log"));
    wait_for(&handle, 20, |event| matches!(event, Event::Snapshot { .. }));

    let found = |total, current| {
        move |event: &Event| {
            matches!(
                event,
                Event::Found { total: t, current: c, .. } if *t == total && *c == current
            )
        }
    };

    handle.find("hola", 0);
    wait_for(&handle, 10, found(2, 0));
    handle.find("hola", 1);
    wait_for(&handle, 10, found(2, 1));
    // Avanzar desde la última vuelve a la primera.
    handle.find("hola", 2);
    wait_for(&handle, 10, found(2, 0));
    handle.find("no existe", 0);
    wait_for(&handle, 10, found(0, -1));
    handle.find("", 0);
    wait_for(&handle, 10, found(0, -1));
    handle.shutdown();
}

#[test]
fn una_descarga_llega_a_la_carpeta_y_avisa_al_terminar() {
    let _slot = slot();
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let base = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let path = head.split_whitespace().nth(1).unwrap_or("/");
            if path.starts_with("/file") {
                let body = "contenido de prueba\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"informe.txt\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            } else {
                let body = "<body><a href=\"/file\">Descargar</a></body>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let downloads = dir.path().join("descargas");
    let handle = shell::engine::spawn_with_options(
        shell::engine::Options {
            initial_urls: vec![base.clone()],
            log_path: dir.path().join("log"),
            profile: None,
            downloads: Some(downloads.clone()),
        },
        std::sync::Arc::new(|| {}),
    );
    wait_for(&handle, 20, |event| matches!(event, Event::Snapshot { .. }));

    handle.navigate(format!("{base}file"));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut finished = None;
    while Instant::now() < deadline {
        match handle.events.recv_timeout(Duration::from_millis(500)) {
            Ok(Event::Download(download)) if download.state == engine_cdp::DownloadState::Done => {
                finished = Some(download);
                break;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("el motor cerró el canal"),
        }
    }
    let download = finished.expect("la descarga debe completarse y avisarse");
    let saved = downloads.join("informe.txt");
    if let Some(path) = download.path {
        assert_eq!(path, saved);
    }
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap().trim(),
        "contenido de prueba"
    );
    handle.shutdown();
}

#[test]
fn un_enlace_con_target_blank_abre_pestana_nueva() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let base = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let path = head.split_whitespace().nth(1).unwrap_or("/");
            let body = if path.starts_with("/dos") {
                "<title>Dos</title><body>segunda</body>"
            } else {
                "<title>Uno</title><body><a href=\"/dos\" target=\"_blank\">Abrir dos</a></body>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(base.clone(), dir.path().join("log"));
    let (first, elements) = first_snapshot(&handle);
    let link = elements
        .iter()
        .find(|element| element.role == "link")
        .expect("el fixture tiene un enlace");
    handle.click(link.x, link.y);
    match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::TabOpened(info) if info.id != first && info.url.ends_with("/dos")),
    ) {
        Event::TabOpened(info) => {
            assert_ne!(info.id, first, "la pestaña nueva debe ser otra");
            assert!(info.url.ends_with("/dos"));
        }
        _ => unreachable!(),
    }
    handle.shutdown();
}

#[test]
fn un_window_open_aparece_como_pestana() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let base = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let path = head.split_whitespace().nth(1).unwrap_or("/");
            let body = if path.starts_with("/tres") {
                "<title>Tres</title><body>popup</body>"
            } else {
                "<title>Uno</title><body><input id=q autofocus><script>q.addEventListener('keydown',e=>{if(e.key==='Enter')window.open('/tres','_blank')})</script></body>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(base.clone(), dir.path().join("log"));
    let (_, elements) = first_snapshot(&handle);
    let field = elements
        .iter()
        .find(|element| element.role == "textbox")
        .expect("el fixture tiene un campo");
    handle.click(field.x, field.y);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Notice(notice) if notice.contains("enfocado")),
    );
    handle.key("Enter", "Enter", 13, 0);
    // El popup lo abre la propia web; el motor lo adopta como pestaña.
    wait_for(
        &handle,
        25,
        |event| matches!(event, Event::Snapshot { info, .. } if info.url.ends_with("/tres")),
    );
    handle.shutdown();
}

#[test]
fn el_clic_central_abre_en_segundo_plano_y_copia_el_enlace() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let base = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..read]);
            let path = head.split_whitespace().nth(1).unwrap_or("/");
            let body = if path.starts_with("/dos") {
                "<title>Dos</title><body>dos</body>"
            } else {
                "<title>Uno</title><body><a href=\"/dos\">Abrir dos</a></body>"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(base.clone(), dir.path().join("log"));
    let (first, elements) = first_snapshot(&handle);
    let link = elements
        .iter()
        .find(|element| element.role == "link")
        .expect("enlace");

    handle.copy_link(link.x, link.y);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::CopyText(_, url) if url.ends_with("/dos")),
    );

    handle.open_link_new_tab(link.x, link.y, true);
    let second = match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::TabOpened(info) if info.id != first && info.url.ends_with("/dos")),
    ) {
        Event::TabOpened(info) => info.id,
        _ => unreachable!(),
    };
    // Al abrirse en segundo plano no debe cambiar la pestaña activa.
    let deadline = Instant::now() + Duration::from_millis(800);
    while Instant::now() < deadline {
        match handle.events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Selected(Some(id))) => assert_ne!(id, second, "no debía seleccionarse"),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.shutdown();
}

fn aprobar_y_esperar(handle: &EngineHandle, hash: String) {
    handle.approve(hash);
    let deadline = Instant::now() + Duration::from_secs(30);
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
            Ok(Event::Receipts { count, .. }) if count >= 1 => receipts = true,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => panic!("el motor cerró el canal"),
        }
    }
}

fn tarjeta_aprobacion(handle: &EngineHandle) -> (String, card::CardView) {
    match wait_for(handle, 20, |event| {
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
    }
}

#[test]
fn tres_recorridos_locales_comparar_rellenar_y_reservar() {
    let server = navegador::FixtureServer::start();
    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(
        server.page_url("shop.html"),
        dir.path().join("recibos.jsonl"),
    );

    // 1) Comparar el importe visible con el que se va a pagar.
    let (_, elements) = first_snapshot(&handle);
    let pay = elements
        .iter()
        .find(|element| element.name.contains("Pagar"))
        .expect("botón Pagar");
    handle.click(pay.x, pay.y);
    let (hash, card) = tarjeta_aprobacion(&handle);
    assert!(
        card.rows.iter().any(|row| row.value.contains("24.90")),
        "la tarjeta debe coincidir con el total visible: {:?}",
        card.rows
    );
    aprobar_y_esperar(&handle, hash);
    let order = server
        .requests()
        .into_iter()
        .find(|request| request.path == "/order")
        .expect("el pedido debe llegar");
    assert!(
        order.body.contains("amount=24.90"),
        "cuerpo: {}",
        order.body
    );

    // 2) Rellenar un formulario y guardar.
    server.clear();
    handle.navigate(server.page_url("settings.html"));
    let elements = match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Snapshot { info, .. } if info.url.ends_with("settings.html")),
    ) {
        Event::Snapshot { elements, .. } => elements,
        _ => unreachable!(),
    };
    let field = elements
        .iter()
        .find(|element| element.role == "textbox")
        .expect("campo de nombre");
    handle.click(field.x, field.y);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Notice(notice) if notice.contains("enfocado")),
    );
    for _ in 0..3 {
        handle.key("Backspace", "Backspace", 8, 0);
    }
    handle.insert_text("Luis");
    std::thread::sleep(Duration::from_millis(300));
    let save = elements
        .iter()
        .find(|element| element.name.contains("Guardar"))
        .expect("botón Guardar");
    handle.click(save.x, save.y);
    let (hash, card) = tarjeta_aprobacion(&handle);
    assert!(
        card.rows.iter().any(|row| row.value.contains("Luis")),
        "la tarjeta debe mostrar el nombre escrito: {:?}",
        card.rows
    );
    aprobar_y_esperar(&handle, hash);
    let profile = server
        .requests()
        .into_iter()
        .find(|request| request.path == "/profile")
        .expect("el perfil debe guardarse");
    assert!(
        profile.body.contains("name=Luis"),
        "cuerpo: {}",
        profile.body
    );

    // 3) Preparar una reserva y aprobarla.
    server.clear();
    handle.navigate(server.page_url("booking.html"));
    let elements = match wait_for(
        &handle,
        20,
        |event| matches!(event, Event::Snapshot { info, .. } if info.url.ends_with("booking.html")),
    ) {
        Event::Snapshot { elements, .. } => elements,
        _ => unreachable!(),
    };
    let reserve = elements
        .iter()
        .find(|element| element.name.contains("Reservar"))
        .expect("botón Reservar");
    handle.click(reserve.x, reserve.y);
    let (hash, card) = tarjeta_aprobacion(&handle);
    assert!(
        card.rows
            .iter()
            .any(|row| row.value.contains("2026-12-01") || row.value.contains("people")),
        "la tarjeta debe mostrar la reserva: {:?}",
        card.rows
    );
    aprobar_y_esperar(&handle, hash);
    let booking = server
        .requests()
        .into_iter()
        .find(|request| request.path == "/booking")
        .expect("la reserva debe llegar");
    assert!(
        booking.body.contains("date=2026-12-01") && booking.body.contains("people=2"),
        "cuerpo: {}",
        booking.body
    );
    handle.shutdown();
}

#[test]
fn un_boton_que_solo_ejecuta_javascript_es_interaccion_local() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("puerto local");
    let base = format!(
        "http://127.0.0.1:{}/",
        listener.local_addr().unwrap().port()
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let _ = read;
            let body = "<title>JS</title><body><button onclick=\"document.body.dataset.pulsado='1'\">Cambiar</button></body>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    let dir = tempfile::tempdir().expect("tempdir");
    let handle = spawn(base, dir.path().join("log"));
    let (_, elements) = first_snapshot(&handle);
    let button = elements
        .iter()
        .find(|element| element.role == "button")
        .expect("botón");
    handle.click(button.x, button.y);
    wait_for(&handle, 30, |event| {
        matches!(
            event,
            Event::Act {
                outcome: ActOutcome::LocalInteraction { .. },
                ..
            }
        )
    });
    handle.shutdown();
}

#[test]
fn el_arrastre_por_coordenadas_css_conserva_el_texto_a_distintas_escalas() {
    let _slot = slot();
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn(
        "data:text/html,<title>Seleccion</title><body style='margin:0'><p style='margin:0; font:20px monospace'>seleccion comprobada</p></body>".into(),
        dir.path().join("recibos.jsonl"),
    );
    let tab = match wait_for(&handle, 20, |event| {
        matches!(event, Event::Selected(Some(_)))
    }) {
        Event::Selected(Some(id)) => id,
        _ => unreachable!(),
    };
    for scale in [1.0, 1.25, 2.0] {
        handle.viewport(768, 576, scale);
        let event = wait_for(&handle, 20, |event| {
            matches!(event, Event::Frame { frame, .. }
                if frame.width == (768.0 * scale) as usize && frame.height == (576.0 * scale) as usize)
        });
        let Event::Frame { frame, .. } = event else {
            unreachable!()
        };
        assert!(
            (frame.css_width - 768.0).abs() < 1.0,
            "ancho CSS a {scale}: {}",
            frame.css_width
        );
        assert!(
            (frame.css_height - 576.0).abs() < 1.0,
            "alto CSS a {scale}: {}",
            frame.css_height
        );
        handle.select_text(tab, [0.0, 12.0], [270.0, 12.0]);
        handle.copy_text(tab);
        assert!(
            matches!(wait_for(&handle, 10, |event| matches!(event, Event::CopyText(..))),
            Event::CopyText(id, text) if id == tab && text == "seleccion comprobada")
        );
    }
    handle.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelar_una_descarga_lenta_no_interrumpe_otra() {
    use engine_cdp::{Browser, DownloadState, Profile};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    let _slot = slot();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in server.incoming().flatten() {
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut request = [0; 2048];
                let size = stream.read(&mut request).unwrap_or(0);
                let request = String::from_utf8_lossy(&request[..size]);
                let header = if request.starts_with("GET /one ") {
                    "Content-Disposition: attachment; filename=one.bin\r\nContent-Length: 524288\r\n"
                } else if request.starts_with("GET /two ") {
                    "Content-Disposition: attachment; filename=two.bin\r\n"
                } else {
                    let body = "<a id='one' href='/one'>Uno</a><br><a id='two' href='/two'>Dos</a>";
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    return;
                };
                if write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n{header}Connection: close\r\n\r\n").is_err() {
                    return;
                }
                for _ in 0..32 {
                    if stream.write_all(&[1; 16384]).is_err() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let browser = Arc::new(
        Browser::launch(Profile::ephemeral().unwrap(), false)
            .await
            .unwrap(),
    );
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    browser
        .watch_downloads(
            dir.path().to_owned(),
            Arc::new(move |event| {
                let _ = send.send(event);
            }),
        )
        .await
        .unwrap();
    let page = browser.new_page().await.unwrap();
    let mut chrome = engine_cdp::Chrome::attach_existing(browser.clone(), page)
        .await
        .unwrap();
    chrome.navigate(&base).await.unwrap();
    let snapshot = chrome.snapshot(1).await.unwrap();
    for element in snapshot
        .elements
        .iter()
        .filter(|element| element.role == "link")
    {
        chrome.click_at(element.x, element.y).await.unwrap();
    }
    let mut canceled = false;
    let mut finished = false;
    let mut unknown_progress = false;
    let mut requested_cancel = false;
    tokio::time::timeout(Duration::from_secs(15), async {
        while !(canceled && finished) {
            let event = events.recv().await.unwrap();
            match (event.name.as_str(), event.state) {
                ("one.bin", DownloadState::Progress { received, total })
                    if received > 0 && received < total && !requested_cancel =>
                {
                    browser.cancel_download(&event.id).await.unwrap();
                    requested_cancel = true;
                }
                ("one.bin", DownloadState::Canceled) => {
                    assert!(event.path.is_none());
                    canceled = true;
                }
                ("two.bin", DownloadState::Progress { received, total: 0 }) if received > 0 => {
                    unknown_progress = true;
                }
                ("two.bin", DownloadState::Done) => finished = true,
                _ => {}
            }
        }
    })
    .await
    .expect("una descarga cancelada y otra terminada");
    assert!(
        unknown_progress,
        "debe llegar progreso aunque falte Content-Length"
    );
    assert!(!dir.path().join("one.bin").exists());
    assert_eq!(
        std::fs::metadata(dir.path().join("two.bin")).unwrap().len(),
        524288
    );
}

#[test]
fn una_aprobacion_caducada_permite_volver_a_revisar_el_formulario() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    let _slot = slot();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let changed = Arc::new(AtomicBool::new(false));
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", server.local_addr().unwrap());
    let server_requests = requests.clone();
    let server_changed = changed.clone();
    std::thread::spawn(move || {
        for mut stream in server.incoming().flatten() {
            let mut bytes = [0; 4096];
            let size = stream.read(&mut bytes).unwrap_or(0);
            let request = String::from_utf8_lossy(&bytes[..size]);
            let body = if request.starts_with("GET /changed ") {
                if server_changed.load(Ordering::SeqCst) {
                    "yes"
                } else {
                    "no"
                }
            } else if request.starts_with("POST /submit ") {
                server_requests.lock().unwrap().push(request.to_string());
                "<title>Enviado</title>"
            } else {
                r#"<form action="/submit" method="post">
                  <input name="nombre" value="primero">
                  <button id="aceptar">Confirmar</button></form>
                  <script>const poll = setInterval(async () => {
                    if (await (await fetch('/changed', {cache:'no-store'})).text() !== 'yes') return;
                    document.querySelector('input').value = 'vigente';
                    document.querySelector('button').textContent = 'Confirmar cambio';
                    document.body.dataset.changed = 'yes';
                    clearInterval(poll);
                  }, 50)</script>"#
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn(base, dir.path().join("log.jsonl"));
    let (_, elements) = first_snapshot(&handle);
    let button = elements
        .iter()
        .find(|element| element.role == "button")
        .unwrap();
    let approve_card = |x, y| {
        handle.click(x, y);
        match wait_for(&handle, 10, |event| {
            matches!(
                event,
                Event::Act {
                    outcome: ActOutcome::ApprovalRequired { .. },
                    ..
                }
            )
        }) {
            Event::Act {
                outcome: ActOutcome::ApprovalRequired { hash, .. },
                ..
            } => hash,
            _ => unreachable!(),
        }
    };

    let first = approve_card(button.x, button.y);
    changed.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(300));
    handle.approve(first);
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Notice(msg) if msg.starts_with("La página cambió")),
    );
    assert!(requests.lock().unwrap().is_empty());

    let _ = approve_card(button.x, button.y);
    handle.deny();
    wait_for(
        &handle,
        10,
        |event| matches!(event, Event::Notice(msg) if msg.starts_with("acción denegada")),
    );
    assert!(requests.lock().unwrap().is_empty());

    handle.approve(approve_card(button.x, button.y));
    wait_for(&handle, 10, |event| {
        matches!(
            event,
            Event::Act {
                outcome: ActOutcome::Executed { .. },
                ..
            }
        )
    });
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains("nombre=vigente"));
    handle.shutdown();
}
