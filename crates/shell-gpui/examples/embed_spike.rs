//! Prueba: reparentar una ventana de Chromium dentro de una ventana GPUI.
//! Se ejecuta bajo X11 (XWayland) y sirve para validar el enfoque nativo.

use std::process::{Child, Command};
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Application, Bounds, Context, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

struct Spike {
    child: Arc<std::sync::Mutex<Option<Child>>>,
    embedded: Arc<std::sync::Mutex<bool>>,
}

impl Render for Spike {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if !*self.embedded.lock().unwrap() {
            let handle = <Window as HasWindowHandle>::window_handle(window).ok();
            if let Some(RawWindowHandle::Xcb(xcb)) = handle.map(|h| h.as_raw()) {
                let xid = xcb.window.get();
                let embedded = self.embedded.clone();
                let mut embedded = embedded.lock().unwrap();
                if !*embedded {
                    if embed(xid) {
                        *embedded = true;
                    }
                }
            }
        }
        div()
            .size_full()
            .bg(rgb(0x202020))
            .child(div().h(px(40.)).bg(rgb(0x111215)).child("spike nativo"))
    }
}

fn main() {
    let url = std::env::args().nth(1).unwrap_or_else(|| "about:blank".into());
    let mut child = Command::new("/usr/bin/chromium")
        .args([
            "--app",
            &url,
            "--no-first-run",
            "--no-default-browser-check",
            "--user-data-dir=/tmp/navegador-spike-perfil",
            "--window-size=900,700",
            "--window-position=0,0",
        ])
        .env("DISPLAY", std::env::var("DISPLAY").unwrap_or_default())
        .env_remove("WAYLAND_DISPLAY")
        .spawn()
        .expect("chromium");
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(120));
        let _ = child.kill();
    });
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1000.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("spike-nativo".into()),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Spike {
                    child: Arc::new(std::sync::Mutex::new(None)),
                    embedded: Arc::new(std::sync::Mutex::new(false)),
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}

fn embed(parent: u32) -> bool {
    let (conn, screen) = match RustConnection::connect(None) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("X11: {error}");
            return false;
        }
    };
    let root = conn.setup().roots[screen].root;
    let tree = match conn.query_tree(root) {
        Ok(value) => value,
        Err(_) => return false,
    };
    for &win in &tree.children {
        let class = conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .map(|reply| String::from_utf8_lossy(&reply.value).to_lowercase())
            .unwrap_or_default();
        let title = conn
            .get_property(false, win, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 512)
            .ok()
            .map(|reply| String::from_utf8_lossy(&reply.value).to_string())
            .unwrap_or_default();
        eprintln!("ventana {win}: class={class:?} title={title:?}");
        if class.contains("chromium") {
            let geometry = conn.get_geometry(win).ok();
            eprintln!("  geometría: {geometry:?}");
            // Reparenta como hija de la ventana GPUI y la coloca bajo la barra.
            if conn.reparent_window(win, parent, 0, 40).is_err() {
                return false;
            }
            let _ = conn.configure_window(
                win,
                &ConfigureWindowAux::new()
                    .x(0)
                    .y(40)
                    .width(1000)
                    .height(720),
            );
            let _ = conn.map_window(win);
            let _ = conn.flush();
            eprintln!("reparentada {win} en {parent}");
            return true;
        }
    }
    let _ = conn.flush();
    false
}
