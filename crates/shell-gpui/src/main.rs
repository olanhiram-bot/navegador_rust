//! Shell con GPUI: la misma sesión auditable, dibujada por GPU.
//!
//! El motor y los recibos viven en el crate `shell`; aquí solo se pinta y se
//! traduce ratón/teclado a comandos. Los frames llegan ya decodificados y se
//! suben como textura; el scroll y la UI no pasan por CPU de egui.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, Application, Bounds, ClipboardItem, Context, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, ObjectFit, RenderImage, ScrollDelta, ScrollWheelEvent, SharedString, Window,
    WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, size,
};
use shell::engine::{EngineHandle, Event, Options, TabId, TabInfo, spawn_with_options};

const TAB_BAR: f32 = 32.0;
const TOOLBAR: f32 = 42.0;
const STATUS: f32 = 22.0;

struct Shell {
    handle: EngineHandle,
    focus: FocusHandle,
    tabs: Vec<TabInfo>,
    active: Option<TabId>,
    address: String,
    editing: bool,
    status: String,
    frame: Option<Arc<RenderImage>>,
    previous_frame: Option<Arc<RenderImage>>,
    css: (f32, f32),
    /// Tamaño del área de página en puntos de ventana.
    page_size: (f32, f32),
    last_viewport: (u32, u32, f64),
    pending: Option<(String, card::CardView)>,
    result: Option<String>,
}

impl Shell {
    fn new(cx: &mut Context<Self>, initial: String) -> Self {
        let profile = shell::estado::data_dir().map(|dir| dir.join("perfil"));
        let log_path = shell::estado::state_dir()
            .map(|dir| dir.join("recibos.jsonl"))
            .unwrap_or_else(|| PathBuf::from("navegador-recibos.jsonl"));
        let options = Options {
            initial_urls: vec![initial.clone()],
            log_path,
            profile,
            downloads: shell::estado::downloads_dir(),
        };
        let handle = spawn_with_options(options, Arc::new(|| {}));
        let focus = cx.focus_handle();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(12))
                    .await;
                let alive = this.update(cx, |shell, cx| {
                    shell.drain(cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        Shell {
            handle,
            focus,
            tabs: Vec::new(),
            active: None,
            address: initial,
            editing: false,
            status: "cargando…".into(),
            frame: None,
            previous_frame: None,
            css: (1.0, 1.0),
            page_size: (1.0, 1.0),
            last_viewport: (0, 0, 0.0),
            pending: None,
            result: None,
        }
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        while let Ok(event) = self.handle.events.try_recv() {
            match event {
                Event::TabOpened(info) => {
                    if !self.tabs.iter().any(|tab| tab.id == info.id) {
                        self.tabs.push(info);
                    }
                }
                Event::TabClosed(id) => {
                    self.tabs.retain(|tab| tab.id != id);
                    if self.active == Some(id) {
                        self.frame = None;
                    }
                }
                Event::Selected(active) => {
                    self.active = active;
                    self.frame = None;
                    match active.and_then(|id| self.tabs.iter().find(|tab| tab.id == id)) {
                        Some(tab) => {
                            self.address = tab.url.clone();
                            self.status = format!("{} · {} elementos", tab.title, tab.elements);
                        }
                        None => self.status = "sin pestañas".into(),
                    }
                }
                Event::Snapshot { tab, info, .. } => {
                    if let Some(known) = self.tabs.iter_mut().find(|known| known.id == tab) {
                        known.url = info.url.clone();
                        known.title = info.title.clone();
                        known.elements = info.elements;
                    }
                    if Some(tab) == self.active {
                        if !self.editing {
                            self.address = info.url.clone();
                        }
                        self.status = format!(
                            "{} · generación {} · {} elementos",
                            if info.title.is_empty() {
                                info.url
                            } else {
                                info.title
                            },
                            info.generation,
                            info.elements
                        );
                    }
                }
                Event::Frame { tab, frame } => {
                    if Some(tab) == self.active {
                        self.css = (frame.css_width as f32, frame.css_height as f32);
                        let image = image::RgbaImage::from_raw(
                            frame.width as u32,
                            frame.height as u32,
                            frame.rgba,
                        );
                        if let Some(image) = image {
                            let render = RenderImage::new(vec![image::Frame::new(image)]);
                            self.previous_frame = self.frame.replace(Arc::new(render));
                        }
                    }
                }
                Event::Act { tab, outcome } => {
                    if Some(tab) != self.active {
                        continue;
                    }
                    match &outcome {
                        session::ActOutcome::ApprovalRequired { hash, card, .. } => {
                            self.pending = Some((hash.clone(), card.clone()));
                            self.status = "esperando tu aprobación".into();
                        }
                        other => {
                            self.pending = None;
                            self.result = Some(other.render_text());
                            self.status = "resultado de la acción".into();
                        }
                    }
                }
                Event::Notice(notice) => self.status = notice,
                Event::Error(error) => self.status = format!("error: {error}"),
                Event::CopyText(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Event::Found { .. }
                | Event::Download(_)
                | Event::Receipts { .. }
                | Event::Handoff { .. } => {}
            }
        }
    }

    fn navigate(&mut self) {
        let url = shell::app::browser_url(&self.address);
        self.address = url.clone();
        self.editing = false;
        self.handle.navigate(url);
    }

    fn page_click(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let (w, h) = self.page_size;
        let x = (f32::from(event.position.x) / w) * self.css.0;
        let y = ((f32::from(event.position.y) - TAB_BAR - TOOLBAR) / h) * self.css.1;
        if x >= 0.0 && y >= 0.0 {
            self.handle.click(x as f64, y as f64);
        }
        cx.notify();
    }

    fn page_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (w, h) = self.page_size;
        let x = (f32::from(event.position.x) / w) * self.css.0;
        let y = ((f32::from(event.position.y) - TAB_BAR - TOOLBAR) / h) * self.css.1;
        let delta = match event.delta {
            ScrollDelta::Pixels(point) => f32::from(point.y),
            ScrollDelta::Lines(point) => point.y * 40.0,
        };
        self.handle.wheel(x as f64, y as f64, -delta as f64);
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let control = keystroke.modifiers.control || keystroke.modifiers.platform;
        let key = keystroke.key.as_str();

        if control {
            match key {
                "l" => {
                    self.editing = true;
                    self.address.clear();
                    cx.notify();
                    return;
                }
                "t" => {
                    self.handle.new_tab(None);
                    return;
                }
                "w" => {
                    if let Some(id) = self.active {
                        self.handle.close_tab(id);
                    }
                    return;
                }
                "r" => {
                    self.handle.reload();
                    return;
                }
                "q" => {
                    cx.quit();
                    return;
                }
                _ => {}
            }
        }

        if self.editing {
            match key {
                "enter" => self.navigate(),
                "escape" => self.editing = false,
                "backspace" => {
                    self.address.pop();
                }
                _ => {
                    if let Some(text) = &keystroke.key_char
                        && !control
                    {
                        self.address.push_str(text);
                    }
                }
            }
            cx.notify();
            return;
        }

        if let Some((hash, _)) = self.pending.clone() {
            match key {
                "enter" => {
                    self.handle.approve(hash);
                    self.pending = None;
                }
                "escape" => {
                    self.handle.deny();
                    self.pending = None;
                }
                _ => {}
            }
            cx.notify();
            return;
        }

        if let Some(text) = &keystroke.key_char
            && !control
            && !keystroke.modifiers.alt
        {
            self.handle.insert_text(text.clone());
            return;
        }
        let special = match key {
            "enter" => Some(("Enter", "Enter", 13)),
            "backspace" => Some(("Backspace", "Backspace", 8)),
            "delete" => Some(("Delete", "Delete", 46)),
            "escape" => Some(("Escape", "Escape", 27)),
            "tab" => Some(("Tab", "Tab", 9)),
            "left" => Some(("ArrowLeft", "ArrowLeft", 37)),
            "up" => Some(("ArrowUp", "ArrowUp", 38)),
            "right" => Some(("ArrowRight", "ArrowRight", 39)),
            "down" => Some(("ArrowDown", "ArrowDown", 40)),
            "home" => Some(("Home", "Home", 36)),
            "end" => Some(("End", "End", 35)),
            "pageup" => Some(("PageUp", "PageUp", 33)),
            "pagedown" => Some(("PageDown", "PageDown", 34)),
            _ => None,
        };
        if let Some((name, code, virtual_key)) = special {
            let mut modifiers = 0;
            if keystroke.modifiers.alt {
                modifiers |= 1;
            }
            if control {
                modifiers |= 2;
            }
            if keystroke.modifiers.platform {
                modifiers |= 4;
            }
            if keystroke.modifiers.shift {
                modifiers |= 8;
            }
            self.handle.key(name, code, virtual_key, modifiers);
        }
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focus.is_focused(window) && !self.editing && self.pending.is_none() {
            window.focus(&self.focus);
        }
        if let Some(old) = self.previous_frame.take() {
            let _ = cx.drop_image(old, Some(window));
        }

        let viewport = window.viewport_size();
        let page_w = f32::from(viewport.width).max(320.0);
        let page_h = (f32::from(viewport.height) - TAB_BAR - TOOLBAR - STATUS).max(240.0);
        self.page_size = (page_w, page_h);
        let scale = 1.0f64;
        let key = (page_w as u32, page_h as u32, scale);
        if key != self.last_viewport {
            self.last_viewport = key;
            self.handle.viewport(key.0, key.1, scale);
        }

        let tabs = self.tabs.clone();
        let active = self.active;
        let editing = self.editing;
        let address = if editing {
            format!("{}|", self.address)
        } else {
            self.address.clone()
        };
        let status = self.status.clone();
        let pending = self.pending.clone();
        let result = self.result.clone();

        let tab_strip = div()
            .h(px(TAB_BAR))
            .w_full()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(rgb(0x0b0c0f))
            .children(tabs.into_iter().map(|tab| {
                let selected = Some(tab.id) == active;
                let color = if selected {
                    rgb(0xffffff)
                } else {
                    rgb(0x9aa0ad)
                };
                let fill = if selected {
                    rgb(0x2d2f36)
                } else {
                    rgb(0x1a1c22)
                };
                let title = if tab.title.is_empty() {
                    tab.url.clone()
                } else {
                    tab.title.clone()
                };
                div()
                    .id(SharedString::from(format!("tab-{}", tab.id)))
                    .h(px(24.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .bg(fill)
                    .text_xs()
                    .text_color(color)
                    .cursor_pointer()
                    .on_click(cx.listener(move |shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.select_tab(tab.id);
                        cx.notify();
                    }))
                    .child(if title.chars().count() > 22 {
                        format!("{}…", title.chars().take(22).collect::<String>())
                    } else {
                        title
                    })
            }))
            .child(
                div()
                    .id("nueva-pestana")
                    .w(px(24.))
                    .h(px(24.))
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(rgb(0x9aa0ad))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.new_tab(None);
                        cx.notify();
                    }))
                    .child("+"),
            );

        let toolbar = div()
            .h(px(TOOLBAR))
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(0x111215))
            .child(
                div()
                    .id("atras")
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded_md()
                    .text_xs()
                    .text_color(rgb(0xcfd3e0))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.back();
                        cx.notify();
                    }))
                    .child("Atrás"),
            )
            .child(
                div()
                    .id("adelante")
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded_md()
                    .text_xs()
                    .text_color(rgb(0xcfd3e0))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.forward();
                        cx.notify();
                    }))
                    .child("Adelante"),
            )
            .child(
                div()
                    .id("recargar")
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .justify_center()
                    .items_center()
                    .rounded_md()
                    .text_xs()
                    .text_color(rgb(0xcfd3e0))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x2a2d36)))
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.handle.reload();
                        cx.notify();
                    }))
                    .child("Recargar"),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(30.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(rgb(0x262832))
                    .text_sm()
                    .text_color(rgb(0xd6d9e2))
                    .id("omnibox")
                    .cursor_text()
                    .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                        shell.editing = true;
                        cx.notify();
                    }))
                    .child(address),
            );

        let loading = self.frame.is_none();
        let mut page = div()
            .relative()
            .flex_1()
            .w_full()
            .bg(rgb(0x0c0d0f))
            .overflow_hidden()
            .id("pagina")
            .on_mouse_down(MouseButton::Left, cx.listener(Self::page_click))
            .on_scroll_wheel(cx.listener(Self::page_wheel));
        if let Some(frame) = self.frame.clone() {
            page = page.child(img(frame).object_fit(ObjectFit::Fill).w_full().h_full());
        } else if loading {
            page = page.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .justify_center()
                    .items_center()
                    .text_color(rgb(0x9aa0ad))
                    .child("cargando la página…"),
            );
        }

        let status_bar = div()
            .h(px(STATUS))
            .w_full()
            .flex()
            .items_center()
            .px_3()
            .bg(rgb(0x0b0c0f))
            .text_xs()
            .text_color(rgb(0x9aa0ad))
            .child(status);

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x0b0c0f))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(tab_strip)
            .child(toolbar)
            .child(page)
            .child(status_bar);

        if let Some((hash, card)) = pending {
            let approve = hash.clone();
            let rows = card.rows.clone();
            let title = card.title.clone();
            let overlay = div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_center()
                .bg(gpui::rgba(0x000000aa))
                .child(
                    div()
                        .w(px(520.))
                        .p_5()
                        .rounded_lg()
                        .bg(rgb(0x20222a))
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_sm()
                        .text_color(rgb(0xe6e8ef))
                        .child(div().text_lg().child(title))
                        .children(rows.into_iter().map(|row| {
                            div()
                                .flex()
                                .justify_between()
                                .gap_4()
                                .child(div().text_color(rgb(0x9aa0ad)).child(row.label))
                                .child(row.value)
                        }))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .mt_2()
                                .child(
                                    div()
                                        .id("aprobar")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(rgb(0x3a5496))
                                        .cursor_pointer()
                                        .on_click(cx.listener(
                                            move |shell, _: &gpui::ClickEvent, _, cx| {
                                                shell.handle.approve(approve.clone());
                                                shell.pending = None;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Aprobar"),
                                )
                                .child(
                                    div()
                                        .id("denegar")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(rgb(0x3a3d47))
                                        .cursor_pointer()
                                        .on_click(cx.listener(
                                            |shell, _: &gpui::ClickEvent, _, cx| {
                                                shell.handle.deny();
                                                shell.pending = None;
                                                cx.notify();
                                            },
                                        ))
                                        .child("Denegar"),
                                ),
                        ),
                );
            root = root.child(overlay);
        }

        if let Some(result) = result {
            let overlay = div()
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_center()
                .bg(gpui::rgba(0x000000aa))
                .child(
                    div()
                        .w(px(560.))
                        .p_5()
                        .rounded_lg()
                        .bg(rgb(0x20222a))
                        .flex()
                        .flex_col()
                        .gap_3()
                        .text_xs()
                        .text_color(rgb(0xe6e8ef))
                        .child(div().text_lg().child("Resultado"))
                        .child(div().whitespace_normal().child(result))
                        .child(
                            div()
                                .id("cerrar-resultado")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(rgb(0x3a3d47))
                                .cursor_pointer()
                                .on_click(cx.listener(|shell, _: &gpui::ClickEvent, _, cx| {
                                    shell.result = None;
                                    cx.notify();
                                }))
                                .child("Cerrar"),
                        ),
                );
            root = root.child(overlay);
        }

        root
    }
}

fn main() {
    let initial = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with("--"))
        .unwrap_or_else(|| "about:blank".to_string());
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.), px(780.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Navegador".into()),
                    ..Default::default()
                }),
                app_id: Some("navegador".into()),
                ..Default::default()
            },
            |_, cx| cx.new(|cx| Shell::new(cx, initial.clone())),
        )
        .expect("no se pudo abrir la ventana");
        cx.activate(true);
    });
}
