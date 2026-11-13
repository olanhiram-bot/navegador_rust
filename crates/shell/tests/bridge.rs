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

    handle.shutdown();
}
