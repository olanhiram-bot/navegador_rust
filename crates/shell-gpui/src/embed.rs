//! Coloca la ventana de Chromium encima del área de página de la shell.
//!
//! GPUI no expone el XID de su ventana (su backend X11 no implementa
//! `HasWindowHandle`), así que en vez de reparentar se usa una ventana X11
//! superpuesta: se marca Chromium como `override_redirect` (el compositor deja
//! de gestionarla), se coloca en coordenadas absolutas sobre la shell y se
//! mantiene al frente. La shell encuentra su propia ventana por WM_CLASS.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub struct Embed {
    conn: RustConnection,
    child: Window,
    shell: Window,
    bounds: (i16, i16, u16, u16),
    absolute: (i16, i16),
    last_shell: Instant,
    visible: bool,
}

impl Embed {
    pub fn attach(pid: u32) -> Result<Self> {
        let (conn, screen) = RustConnection::connect(None).context("conectar a X11")?;
        let root = conn.setup().roots[screen].root;
        let child = find_window(&conn, root, pid, "chromium")
            .context("no se encontró la ventana de Chromium")?;
        let shell = find_window(&conn, root, 0, "navegador")
            .context("no se encontró la ventana de la shell")?;

        // Fuera del gestor de ventanas: se coloca a mano sobre la shell.
        conn.unmap_window(child).ok();
        conn.change_window_attributes(
            child,
            &ChangeWindowAttributesAux::new().override_redirect(1),
        )
        .context("marcar Chromium como override-redirect")?;
        conn.map_window(child).ok();
        conn.flush().ok();

        Ok(Self {
            conn,
            child,
            shell,
            bounds: (0, 0, 1, 1),
            absolute: (0, 0),
            last_shell: Instant::now(),
            visible: true,
        })
    }

    /// Coloca Chromium en coordenadas relativas a la shell y la trae al frente.
    pub fn place(&mut self, x: i16, y: i16, width: u16, height: u16) {
        if (x, y, width, height) == self.bounds && self.last_shell.elapsed() < Duration::from_millis(250)
        {
            return;
        }
        self.bounds = (x, y, width, height);
        self.last_shell = Instant::now();
        let shell = self
            .conn
            .get_geometry(self.shell)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|geometry| (geometry.x, geometry.y))
            .unwrap_or(self.absolute);
        self.absolute = shell;
        let absolute_x = shell.0.saturating_add(x);
        let absolute_y = shell.1.saturating_add(y);
        let _ = self.conn.configure_window(
            self.child,
            &ConfigureWindowAux::new()
                .x(i32::from(absolute_x))
                .y(i32::from(absolute_y))
                .width(u32::from(width))
                .height(u32::from(height))
                .stack_mode(StackMode::ABOVE),
        );
        let _ = self.conn.flush();
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            let _ = self.conn.map_window(self.child);
        } else {
            let _ = self.conn.unmap_window(self.child);
        }
        let _ = self.conn.flush();
    }
}

fn find_window(conn: &RustConnection, root: Window, pid: u32, class: &str) -> Result<Window> {
    let net_pid = conn
        .intern_atom(false, b"_NET_WM_PID")
        .context("átomo _NET_WM_PID")?
        .reply()
        .context("leer _NET_WM_PID")?
        .atom;
    let tree = conn.query_tree(root).context("query_tree")?.reply()?;
    let mut fallback = None;
    for &win in &tree.children {
        let wm_class = conn
            .get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| String::from_utf8_lossy(&reply.value).to_lowercase())
            .unwrap_or_default();
        if !wm_class.contains(class) {
            continue;
        }
        if pid != 0 {
            let window_pid = conn
                .get_property(false, win, net_pid, AtomEnum::CARDINAL, 0, 1)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().and_then(|mut values| values.next()));
            if window_pid == Some(pid) {
                return Ok(win);
            }
        }
        fallback = Some(win);
    }
    match fallback {
        Some(win) => Ok(win),
        None => bail!("ventana «{class}» no encontrada"),
    }
}
