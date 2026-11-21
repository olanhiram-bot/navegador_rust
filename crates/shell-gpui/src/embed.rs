//! Incrusta la ventana de Chromium con una ventana contenedora propia.
//!
//! En X11/XWayland se crea una ventana `override-redirect` encima del área de
//! página de la shell y se reparenta dentro el cliente de Chromium. Así el
//! gestor de ventanas no toca ninguna de las dos y la página se dibuja de
//! forma nativa (sin JPEG).

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub struct Embed {
    conn: RustConnection,
    child: Window,
    container: Window,
    shell: Window,
    bounds: (i16, i16, u16, u16),
    last_move: Instant,
    visible: bool,
}

impl Embed {
    /// Busca la shell y Chromium, crea el contenedor y reparenta el cliente.
    pub fn attach(pid: u32) -> Result<Self> {
        let (conn, screen) = RustConnection::connect(None).context("conectar a X11")?;
        let root = conn.setup().roots[screen].root;
        let shell = find_window(&conn, root, 0, "navegador")
            .context("no se encontró la ventana de la shell (¿X11?)")?;
        let child = find_window(&conn, root, pid, "chromium")
            .context("no se encontró la ventana de Chromium")?;

        let container = conn.generate_id().context("generar id X11")?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            container,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0x00101114)
                .event_mask(EventMask::EXPOSURE),
        )
        .context("crear el contenedor X11")?;

        // El gestor puede haber envuelto a Chromium en un marco: se oculta para
        // que no quede un cascarón vacío.
        if let Ok(cookie) = conn.query_tree(child)
            && let Ok(tree) = cookie.reply()
            && tree.parent != root
        {
            conn.unmap_window(tree.parent).ok();
        }
        conn.unmap_window(child).ok();
        conn.reparent_window(child, container, 0, 0)
            .context("reparentar Chromium en el contenedor")?;
        conn.map_window(child).ok();
        conn.map_window(container).ok();
        conn.flush().context("volcar X11")?;

        Ok(Self {
            conn,
            child,
            container,
            shell,
            bounds: (0, 0, 1, 1),
            last_move: Instant::now(),
            visible: true,
        })
    }

    /// Coloca el contenedor sobre el área de página (coordenadas de la shell).
    pub fn place(&mut self, x: i16, y: i16, width: u16, height: u16) {
        if (x, y, width, height) == self.bounds
            && self.last_move.elapsed() < Duration::from_millis(200)
        {
            return;
        }
        self.bounds = (x, y, width, height);
        self.last_move = Instant::now();
        let origin = self
            .conn
            .translate_coordinates(self.shell, self.conn.setup().roots[0].root, 0, 0)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| (reply.dst_x, reply.dst_y))
            .unwrap_or((0, 0));
        let absolute_x = origin.0.saturating_add(x);
        let absolute_y = origin.1.saturating_add(y);

        let _ = self.conn.configure_window(
            self.container,
            &ConfigureWindowAux::new()
                .x(i32::from(absolute_x))
                .y(i32::from(absolute_y))
                .width(u32::from(width))
                .height(u32::from(height))
                .stack_mode(StackMode::ABOVE),
        );
        let _ = self.conn.configure_window(
            self.child,
            &ConfigureWindowAux::new()
                .width(u32::from(width))
                .height(u32::from(height)),
        );
        let _ = self.conn.flush();
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            let _ = self.conn.map_window(self.container);
            let _ = self.conn.configure_window(
                self.container,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
        } else {
            let _ = self.conn.unmap_window(self.container);
        }
        let _ = self.conn.flush();
    }
}

fn class_of(conn: &RustConnection, win: Window) -> String {
    conn.get_property(false, win, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| String::from_utf8_lossy(&reply.value).to_lowercase())
        .unwrap_or_default()
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
        let wm_class = class_of(conn, win);
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
    fallback.with_context(|| format!("no se encontró la ventana «{class}» (pid {pid})"))
}
