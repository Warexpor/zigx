mod gfx;

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::gfx::Gfx;
use zigx::*;

#[derive(Debug)]
enum UserEvent {
    Sample,
}

struct App {
    hub: Hub,
    state: AppState,
    startup: Vec<StartupEntry>,
    mouse: [f32; 2],
    mods: ModifiersState,
    hits: Vec<Hit>,
    hover: Option<HitKind>,
    list_rect: Option<Rect>,
    detail_rect: Option<Rect>,
    startup_rect: Option<Rect>,
    settings_rect: Option<Rect>,
    cursor: CursorIcon,
    /// Interface zoom as drawn; glides to `state.ui_scale`.
    zoom: Spring,
    zoom_at: Instant,
    gfx: Option<Gfx>,
    window: Option<Arc<Window>>,
}

impl App {
    fn new(hub: Hub, state: AppState) -> Self {
        Self {
            hub,
            startup: load_startup(),
            mouse: [0.0, 0.0],
            mods: ModifiersState::empty(),
            hits: Vec::new(),
            hover: None,
            list_rect: None,
            detail_rect: None,
            startup_rect: None,
            settings_rect: None,
            cursor: CursorIcon::Default,
            zoom: Spring::new(state.ui_scale),
            zoom_at: Instant::now(),
            state,
            gfx: None,
            window: None,
        }
    }

    fn zooming(&self) -> bool {
        self.zoom.value != self.state.ui_scale
    }

    fn step_zoom(&mut self) {
        let now = Instant::now();
        let target = self.state.ui_scale;
        if !self.state.settings.animations {
            self.zoom = Spring::new(target);
        } else if self.zooming() {
            // The first step after a still stretch is one frame, not the gap.
            let dt = if self.zoom.vel == 0.0 {
                1.0 / 60.0
            } else {
                now.saturating_duration_since(self.zoom_at)
                    .as_secs_f32()
                    .min(0.05)
            };
            self.zoom.step(target, ZOOM, dt, 0.0005);
        }
        self.zoom_at = now;
    }

    fn sync_size(&mut self) {
        let Some(window) = &self.window else { return };
        let scale = pixel_scale(window, self.zoom.value);
        if scale <= 0.0 {
            return;
        }
        let size = window.inner_size();
        self.state.width = size.width as f32 / scale;
        self.state.height = size.height as f32 / scale;
    }

    fn paint(&mut self) {
        self.step_zoom();
        self.sync_size();
        expire(&mut self.state);
        let snap = self.hub.load();
        let draw = build(&mut self.state, &snap, &self.startup, self.mouse);
        self.hits = draw.hits.clone();
        self.list_rect = draw.list_rect;
        self.detail_rect = draw.detail_rect;
        self.startup_rect = draw.startup_rect;
        self.settings_rect = draw.settings_rect;
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(gfx) = self.gfx.as_mut() {
            let scale = pixel_scale(&window, self.zoom.value);
            gfx.render(&window, &draw, scale);
        }
    }

    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn apply(&mut self, effects: Vec<Effect>, event_loop: &ActiveEventLoop) {
        for effect in effects {
            match effect {
                Effect::Exit => event_loop.exit(),
                Effect::Minimize => {
                    if let Some(window) = &self.window {
                        window.set_minimized(true);
                    }
                }
                Effect::DragWindow => {
                    if let Some(window) = &self.window {
                        let _ = window.drag_window();
                    }
                }
                Effect::Signal(pids, sig) => self.signal(&pids, sig),
                Effect::OpenLocation(pid) => self.open_location(pid),
                Effect::Copy(text) => {
                    let what = if text.contains(' ') { "PIDs" } else { "PID" };
                    self.copy(&text, what);
                }
                Effect::CopyCommand(pid) => match read_cmdline(pid) {
                    Some(cmd) => self.copy(&cmd, "command line"),
                    None => self.notify("Command line is not readable", 4),
                },
                Effect::FlipStartup(index) => self.flip_startup(index),
                Effect::Persist => {
                    self.hub.set_period(if self.state.paused {
                        0
                    } else {
                        self.state.settings.speed.ms()
                    });
                    if let Err(err) = save_ui(&self.state) {
                        self.notify(format!("Could not save settings: {err}"), 6);
                    }
                }
                Effect::Notify(label) => self.notify(label, 3),
            }
        }
    }

    fn notify(&mut self, label: impl Into<String>, secs: u64) {
        self.state.notice = Some(Notice {
            until: Instant::now() + Duration::from_secs(secs),
            label: label.into(),
        });
    }

    fn signal(&mut self, pids: &[i32], sig: Sig) {
        let asked = pids.len();
        let n = send_signal(pids, sig);
        let (name, verb) = match sig {
            Sig::Term => ("SIGTERM", "sent"),
            Sig::Kill => ("SIGKILL", "sent"),
            Sig::Stop => ("Suspend", "requested"),
            Sig::Cont => ("Resume", "requested"),
        };
        let label = match (n, asked) {
            (0, _) => "Could not signal the selected process".to_string(),
            (1, 1) => format!("{name} {verb}"),
            (n, asked) if n == asked => format!("{name} {verb} for {n} processes"),
            (n, asked) => format!("{name} {verb} for {n} of {asked} processes"),
        };
        if matches!(sig, Sig::Term | Sig::Kill) {
            // Dead PIDs can be recycled; do not leave them armed.
            self.state.selected.clear();
            self.state.pinned.clear();
            self.state.anchor = None;
        }
        self.notify(label, 4);
    }

    fn open_location(&mut self, pid: i32) {
        let dir = std::fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|exe| exe.parent().map(|p| p.to_path_buf()));
        let Some(dir) = dir else {
            self.notify("Executable path is not readable", 4);
            return;
        };
        match spawn_detached(Command::new("xdg-open").arg(&dir)) {
            Ok(()) => self.notify(format!("Opening {}", dir.display()), 3),
            Err(_) => self.notify("xdg-open is not available", 4),
        }
    }

    fn copy(&mut self, text: &str, what: &str) {
        let tools: [(&str, &[&str]); 3] = [
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ];
        for (bin, args) in tools {
            let child = Command::new(bin)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else { continue };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            self.notify(format!("Copied {what}"), 3);
            return;
        }
        self.notify("No clipboard tool found (install wl-clipboard)", 5);
    }

    fn flip_startup(&mut self, index: usize) {
        let Some(entry) = self.startup.get(index) else {
            return;
        };
        let enable = !entry.enabled;
        let name = entry.name.clone();
        let path = entry.path.clone();
        let system_path = entry.system_path.clone();
        if let Err(err) = write_enabled(&path, system_path.as_deref(), enable) {
            self.notify(format!("Could not update {name}: {err}"), 6);
        }
        self.startup = load_startup();
    }

    fn pointer(&self, window: &Window, x: f64, y: f64) -> [f32; 2] {
        let scale = pixel_scale(window, self.zoom.value);
        if scale <= 0.0 {
            return [x as f32, y as f32];
        }
        [x as f32 / scale, y as f32 / scale]
    }
}

/// Monitor DPI times the user zoom. Layout is in design pixels; this maps them to the framebuffer.
fn pixel_scale(window: &Window, ui_scale: f32) -> f32 {
    window.scale_factor() as f32 * ui_scale
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // ~70% of the primary monitor, keeping its aspect ratio (fallback 16:9).
        let (w, h) = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .map(|m| {
                let px = m.size();
                let scale = m.scale_factor().max(0.1);
                (
                    (px.width as f64 / scale) * 0.70,
                    (px.height as f64 / scale) * 0.70,
                )
            })
            .unwrap_or((1344.0, 756.0));
        let mut attrs = WindowAttributes::default()
            .with_title("ZIGX")
            .with_inner_size(LogicalSize::new(w, h))
            .with_min_inner_size(LogicalSize::new(420.0, 320.0))
            .with_transparent(true)
            .with_decorations(false);
        attrs =
            winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "zigx", "zigx");
        attrs = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "zigx", "zigx");
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        self.gfx = Some(Gfx::new(window.clone(), event_loop));
        self.window = Some(window);
        self.redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
        self.redraw();
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // A timed wake (see `wake_at`) means something is due to change.
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.page == Page::Performance || animating(&self.state) || self.zooming() {
            // Keep graph playback and transitions advancing at display rate.
            self.redraw();
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(16),
            ));
        } else if let Some(at) = wake_at(&self.state) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(at));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.redraw(),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::ModifiersChanged(mods) => self.mods = mods.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = self.pointer(&window, position.x, position.y);
                let dragging = on_move(&mut self.state, self.mouse[0], self.mouse[1]);
                let kind = hit_at(&self.hits, self.mouse[0], self.mouse[1]);
                let cursor = match (&self.state.drag, kind) {
                    (Some(Drag::Scroll { .. }), _) | (_, Some(HitKind::Scroll(_))) => {
                        CursorIcon::NsResize
                    }
                    (_, Some(HitKind::DragNav | HitKind::DragSub)) => CursorIcon::EwResize,
                    (_, Some(HitKind::Search)) => CursorIcon::Text,
                    (_, Some(HitKind::DragWindow | HitKind::MenuPanel)) => CursorIcon::Default,
                    (_, Some(_)) => CursorIcon::Pointer,
                    (_, None) => CursorIcon::Default,
                };
                if cursor != self.cursor {
                    window.set_cursor(cursor);
                    self.cursor = cursor;
                }
                if dragging || kind != self.hover {
                    self.hover = kind;
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Right {
                    if state == ElementState::Pressed {
                        match hit_at(&self.hits, self.mouse[0], self.mouse[1]) {
                            Some(HitKind::Proc { pid }) if self.state.page == Page::Processes => {
                                open_menu(&mut self.state, pid, self.mouse);
                            }
                            _ => close_menu(&mut self.state),
                        }
                        self.redraw();
                    }
                    return;
                }
                if button != MouseButton::Left {
                    return;
                }
                if state == ElementState::Pressed {
                    if let Some(kind) = hit_at(&self.hits, self.mouse[0], self.mouse[1]) {
                        let effects = on_press(
                            &mut self.state,
                            kind,
                            self.mods.control_key(),
                            self.mods.shift_key(),
                            self.mouse,
                        );
                        note_drag_origin(&mut self.state, self.mouse[0], self.mouse[1]);
                        self.apply(effects, event_loop);
                    } else if self.state.menu.is_some() {
                        close_menu(&mut self.state);
                    } else {
                        self.state.search_focused = false;
                        // Empty chrome (no hit target) clears a process selection.
                        if self.state.page == Page::Processes && !self.state.selected.is_empty() {
                            clear_selection(&mut self.state);
                        }
                    }
                } else {
                    let effects = on_release(&mut self.state);
                    self.apply(effects, event_loop);
                }
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 48.0,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                let [x, y] = self.mouse;
                let panes = [
                    (self.list_rect, ScrollBar::Processes),
                    (self.detail_rect, ScrollBar::Performance),
                    (self.startup_rect, ScrollBar::Startup),
                    (self.settings_rect, ScrollBar::Settings),
                ];
                let over = panes
                    .into_iter()
                    .find(|(r, _)| r.is_some_and(|r| r.contains(x, y)))
                    .map(|(_, which)| which);
                on_wheel(&mut self.state, over, dy);
                self.redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                let Some(key) = map_key(event.logical_key) else {
                    return;
                };
                let effects = on_key(&mut self.state, key, self.mods.control_key());
                self.apply(effects, event_loop);
                self.redraw();
            }
            _ => {}
        }
    }
}

fn map_key(key: Key) -> Option<KeyIn> {
    Some(match key {
        Key::Named(NamedKey::Escape) => KeyIn::Escape,
        Key::Named(NamedKey::Backspace) => KeyIn::Backspace,
        Key::Named(NamedKey::Delete) => KeyIn::Delete,
        Key::Named(NamedKey::Enter) => KeyIn::Enter,
        Key::Named(NamedKey::PageUp) => KeyIn::PageUp,
        Key::Named(NamedKey::PageDown) => KeyIn::PageDown,
        Key::Named(NamedKey::ArrowUp) => KeyIn::Up,
        Key::Named(NamedKey::ArrowDown) => KeyIn::Down,
        Key::Named(NamedKey::Space) => KeyIn::Char(' '),
        Key::Character(s) => KeyIn::Char(s.chars().next()?),
        _ => return None,
    })
}

/// Run a helper without blocking the UI, and reap it so it never lingers as a zombie.
fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// `/proc/<pid>/cmdline` with NUL separators turned into spaces.
fn read_cmdline(pid: i32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let parts: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    let proxy = event_loop.create_proxy();
    let mut state = AppState::new(1240.0, 780.0);
    load_ui(&mut state);
    let hub = spawn(state.settings.speed.ms(), move || {
        let _ = proxy.send_event(UserEvent::Sample);
    });
    let mut app = App::new(hub, state);
    event_loop.run_app(&mut app).expect("run");
}
