mod gfx;

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId, WindowLevel};

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
    cursor: CursorIcon,
    gfx: Option<Gfx>,
    window: Option<Arc<Window>>,
}

impl App {
    fn new(hub: Hub) -> Self {
        let mut state = AppState::new(1240.0, 780.0);
        load_ui(&mut state);
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
            cursor: CursorIcon::Default,
            state,
            gfx: None,
            window: None,
        }
    }

    fn sync_size(&mut self) {
        let Some(window) = &self.window else { return };
        let scale = window.scale_factor() as f32;
        if scale <= 0.0 {
            return;
        }
        let size = window.inner_size();
        self.state.width = size.width as f32 / scale;
        self.state.height = size.height as f32 / scale;
    }

    fn paint(&mut self) {
        self.sync_size();
        expire(&mut self.state);
        let snap = self.hub.load();
        let draw = build(&mut self.state, &snap, &self.startup, self.mouse);
        self.hits = draw.hits.clone();
        self.list_rect = draw.list_rect;
        self.detail_rect = draw.detail_rect;
        self.startup_rect = draw.startup_rect;
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(gfx) = self.gfx.as_mut() {
            gfx.render(&window, &draw);
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
                Effect::AlwaysOnTop(on) => {
                    if let Some(window) = &self.window {
                        window.set_window_level(if on {
                            WindowLevel::AlwaysOnTop
                        } else {
                            WindowLevel::Normal
                        });
                    }
                }
                Effect::DragWindow => {
                    if let Some(window) = &self.window {
                        let _ = window.drag_window();
                    }
                }
                Effect::Kill(pids) => {
                    let n = terminate(&pids);
                    self.state.undo = Some(Undo {
                        until: Instant::now() + Duration::from_secs(4),
                        label: format!("SIGTERM sent to {n}"),
                        revert: None,
                    });
                }
                Effect::FlipStartup(index) => self.flip_startup(index),
                Effect::UndoStartup => self.undo_startup(),
                Effect::Persist => {
                    let _ = save_ui(&self.state);
                }
            }
        }
    }

    fn flip_startup(&mut self, index: usize) {
        let Some(entry) = self.startup.get(index) else {
            return;
        };
        let enable = !entry.enabled;
        let name = entry.name.clone();
        let path = entry.path.clone();
        match write_enabled(&path, enable) {
            Ok(revert) => {
                self.state.undo = Some(Undo {
                    until: Instant::now() + Duration::from_secs(10),
                    label: format!("{name} {}", if enable { "on" } else { "off" }),
                    revert: Some(revert),
                });
                self.startup = load_startup();
            }
            Err(err) => {
                self.state.undo = Some(Undo {
                    until: Instant::now() + Duration::from_secs(6),
                    label: format!("Could not update {name}: {err}"),
                    revert: None,
                });
            }
        }
    }

    fn undo_startup(&mut self) {
        let Some(undo) = self.state.undo.take() else {
            return;
        };
        if let Some(revert) = undo.revert {
            if let Err(err) = restore_startup(&revert) {
                self.state.undo = Some(Undo {
                    until: Instant::now() + Duration::from_secs(6),
                    label: format!("Undo failed: {err}"),
                    revert: None,
                });
            }
            self.startup = load_startup();
        }
    }

    fn pointer(&self, window: &Window, x: f64, y: f64) -> [f32; 2] {
        let scale = window.scale_factor() as f32;
        if scale <= 0.0 {
            return [x as f32, y as f32];
        }
        [x as f32 / scale, y as f32 / scale]
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = WindowAttributes::default()
            .with_title("ZIGX")
            .with_inner_size(LogicalSize::new(1240.0, 780.0))
            .with_min_inner_size(LogicalSize::new(420.0, 320.0))
            .with_transparent(true)
            .with_decorations(false);
        attrs =
            winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "zigx", "zigx");
        attrs = winit::platform::x11::WindowAttributesExtX11::with_name(attrs, "zigx", "zigx");
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        if self.state.always_on_top {
            window.set_window_level(WindowLevel::AlwaysOnTop);
        }
        self.gfx = Some(Gfx::new(window.clone(), event_loop));
        self.window = Some(window);
        self.redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: UserEvent) {
        self.redraw();
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
                let dragging = on_move(&mut self.state, self.mouse[0]);
                let kind = hit_at(&self.hits, self.mouse[0], self.mouse[1]);
                let cursor = match kind {
                    Some(HitKind::DragNav | HitKind::DragSub) => CursorIcon::EwResize,
                    Some(HitKind::Search) => CursorIcon::Text,
                    Some(HitKind::DragWindow) => CursorIcon::Default,
                    Some(_) => CursorIcon::Pointer,
                    None => CursorIcon::Default,
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
                        );
                        note_drag_origin(&mut self.state, self.mouse[0]);
                        self.apply(effects, event_loop);
                    } else {
                        self.state.search_focused = false;
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
                let x = self.mouse[0];
                let y = self.mouse[1];
                on_wheel(
                    &mut self.state,
                    self.list_rect.is_some_and(|r| r.contains(x, y)),
                    self.detail_rect.is_some_and(|r| r.contains(x, y)),
                    self.startup_rect.is_some_and(|r| r.contains(x, y)),
                    dy,
                );
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
        Key::Character(s) => KeyIn::Char(s.chars().next()?),
        _ => return None,
    })
}

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    let proxy = event_loop.create_proxy();
    let hub = spawn(move || {
        let _ = proxy.send_event(UserEvent::Sample);
    });
    let mut app = App::new(hub);
    event_loop.run_app(&mut app).expect("run");
}
