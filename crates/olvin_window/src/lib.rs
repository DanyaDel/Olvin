//! Platform layer: window, input and the main loop (winit).
//! Publishes [`SurfaceTarget`] for renderers and [`Input`] for game code.

mod input;
mod settings;

pub use input::Input;
pub use settings::{DEFAULT_PATH, Monitors, Resolution, WindowMode, WindowSettings};

/// What gameplay wants the OS cursor to do; the window applies changes after
/// each frame. `grabbed` locks the cursor to the window (falls back to
/// confining it where locking is unsupported) for mouse look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorOptions {
    pub grabbed: bool,
    pub visible: bool,
}

impl Default for CursorOptions {
    fn default() -> Self {
        Self {
            grabbed: false,
            visible: true,
        }
    }
}
pub use winit::event::MouseButton;
pub use winit::keyboard::KeyCode;

use std::sync::Arc;

use olvin_core::{App, Plugin, Stage, SurfaceTarget};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::monitor::MonitorHandle;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

#[derive(Clone, Debug)]
pub struct WindowPlugin {
    pub title: String,
    /// Initial size (logical pixels in a window, video mode in exclusive fullscreen).
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
    /// Initial mode. At run time change [`WindowSettings`] instead; to start
    /// from a saved file use [`WindowPlugin::with_settings`].
    pub mode: WindowMode,
    /// Whether the window is shown when created (off for smoke tests).
    pub visible: bool,
    /// Lets the event loop be created off the main thread (X11 and Wayland
    /// only, ignored elsewhere). For tests: libtest runs them on workers.
    pub any_thread: bool,
}

impl WindowPlugin {
    /// Takes size and mode from `settings` (e.g. loaded from [`DEFAULT_PATH`]).
    pub fn with_settings(mut self, settings: WindowSettings) -> Self {
        self.width = settings.width;
        self.height = settings.height;
        self.mode = settings.mode;
        self
    }

    fn settings(&self) -> WindowSettings {
        WindowSettings {
            mode: self.mode,
            width: self.width,
            height: self.height,
        }
    }
}

impl Default for WindowPlugin {
    fn default() -> Self {
        Self {
            title: "Olvin".into(),
            width: 1280,
            height: 720,
            resizable: true,
            mode: WindowMode::Windowed,
            visible: true,
            any_thread: false,
        }
    }
}

impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Input>()
            .init_resource::<CursorOptions>()
            .init_resource::<Monitors>()
            .insert_resource(self.settings())
            .add_system(Stage::Last, |res| res.fetch_mut::<Input>().end_frame());
        let config = self.clone();
        app.set_runner(move |app| run(app, config));
    }
}

fn build_event_loop(any_thread: bool) -> Result<EventLoop<()>, winit::error::EventLoopError> {
    let mut builder = EventLoop::builder();
    #[cfg(all(
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "redox")
    ))]
    if any_thread {
        use winit::platform::wayland::EventLoopBuilderExtWayland;
        use winit::platform::x11::EventLoopBuilderExtX11;
        EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
        EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    }
    #[cfg(not(all(
        unix,
        not(target_vendor = "apple"),
        not(target_os = "android"),
        not(target_os = "redox")
    )))]
    let _ = any_thread;
    builder.build()
}

fn run(app: App, config: WindowPlugin) {
    let event_loop = match build_event_loop(config.any_thread) {
        Ok(event_loop) => event_loop,
        Err(err) => {
            log::error!("could not create the event loop (no display?): {err}");
            return;
        }
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner {
        app,
        config,
        window: None,
        cursor: CursorOptions::default(),
        applied: None,
    };
    if let Err(err) = event_loop.run_app(&mut runner) {
        log::error!("event loop error: {err}");
    }
}

struct Runner {
    app: App,
    config: WindowPlugin,
    window: Option<Arc<Window>>,
    /// Last cursor state applied to the window.
    cursor: CursorOptions,
    /// Last [`WindowSettings`] put on the window (or seen from it after a
    /// user resize), so only real changes are applied.
    applied: Option<WindowSettings>,
}

/// The winit fullscreen request for `settings` on `monitor`.
fn fullscreen_for(settings: &WindowSettings, monitor: Option<MonitorHandle>) -> Option<Fullscreen> {
    match settings.mode {
        WindowMode::Windowed => None,
        WindowMode::Borderless => Some(Fullscreen::Borderless(monitor)),
        WindowMode::Exclusive => {
            let Some(monitor) = monitor else {
                return Some(Fullscreen::Borderless(None));
            };
            let modes: Vec<_> = monitor.video_modes().collect();
            let table = Monitors {
                modes: modes.iter().map(resolution).collect(),
            };
            match table.closest(settings.width, settings.height) {
                Some(i) => Some(Fullscreen::Exclusive(modes[i].clone())),
                None => Some(Fullscreen::Borderless(Some(monitor))),
            }
        }
    }
}

fn resolution(mode: &winit::monitor::VideoModeHandle) -> Resolution {
    let size = mode.size();
    Resolution {
        width: size.width,
        height: size.height,
        refresh_mhz: mode.refresh_rate_millihertz(),
    }
}

impl Runner {
    /// Puts [`WindowSettings`] on the window when they changed.
    fn apply_settings(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(wanted) = self.app.resources.get::<WindowSettings>().map(|s| *s) else {
            return;
        };
        let was = self.applied.replace(wanted);
        if was == Some(wanted) {
            return;
        }
        let monitor = window
            .current_monitor()
            .or_else(|| window.primary_monitor());
        let target = fullscreen_for(&wanted, monitor);
        let size = |s: &WindowSettings| (s.width, s.height);
        let size_changed = was.is_none_or(|w| size(&w) != size(&wanted));
        let mode_changed = was.is_none_or(|w| w.mode != wanted.mode);
        // Exclusive fullscreen picks its video mode from the size.
        if mode_changed || (wanted.mode == WindowMode::Exclusive && size_changed) {
            window.set_fullscreen(target);
        }
        let left_fullscreen = was.is_some_and(|w| w.mode.is_fullscreen());
        if !wanted.mode.is_fullscreen() && (size_changed || left_fullscreen) {
            // The size may be refused (tiling WMs); the Resized event then
            // reports what the window really got.
            let _ = window.request_inner_size(LogicalSize::new(wanted.width, wanted.height));
        }
    }

    fn publish_monitors(&self, window: &Window) {
        let Some(monitor) = window
            .current_monitor()
            .or_else(|| window.primary_monitor())
        else {
            return;
        };
        let modes = monitor.video_modes().map(|m| resolution(&m)).collect();
        *self.app.resources.fetch_mut::<Monitors>() = Monitors { modes };
    }

    fn apply_cursor(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(wanted) = self.app.resources.get::<CursorOptions>().map(|c| *c) else {
            return;
        };
        if wanted == self.cursor {
            return;
        }
        if wanted.grabbed != self.cursor.grabbed {
            let result = if wanted.grabbed {
                window
                    .set_cursor_grab(CursorGrabMode::Locked)
                    .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
            } else {
                window.set_cursor_grab(CursorGrabMode::None)
            };
            if let Err(err) = result {
                log::warn!("cursor grab: {err}");
            }
        }
        if wanted.visible != self.cursor.visible {
            window.set_cursor_visible(wanted.visible);
        }
        self.cursor = wanted;
    }
}

impl ApplicationHandler for Runner {
    // Windows may only be created once the loop is running (and on Android,
    // only after `resumed`), so startup systems run from here.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let wanted = self
            .app
            .resources
            .get::<WindowSettings>()
            .map_or(self.config.settings(), |s| *s);
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next());
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(wanted.width, wanted.height))
            .with_resizable(self.config.resizable)
            .with_visible(self.config.visible)
            .with_fullscreen(fullscreen_for(&wanted, monitor));
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                log::error!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.app.insert_resource(
            SurfaceTarget::new(window.clone(), (size.width, size.height))
                .with_scale_factor(window.scale_factor() as f32),
        );
        self.applied = Some(wanted);
        self.publish_monitors(&window);
        self.app.startup();
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let res = &self.app.resources;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                res.fetch_mut::<SurfaceTarget>().size = (size.width, size.height);
                // The user dragged the window edge: remember that size, so
                // saving the settings keeps it and it is not "corrected" back.
                if let Some(window) = &self.window
                    && window.fullscreen().is_none()
                    && let Some(mut settings) = res.get_mut::<WindowSettings>()
                    && settings.mode == WindowMode::Windowed
                {
                    let logical = size.to_logical::<u32>(window.scale_factor());
                    settings.set_resolution(logical.width, logical.height);
                    self.applied = Some(*settings);
                }
            }
            // Moved to a monitor with another display scale (a Resized follows).
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                res.fetch_mut::<SurfaceTarget>().scale_factor = scale_factor as f32;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && !event.repeat
                {
                    res.fetch_mut::<Input>()
                        .set_key(code, event.state.is_pressed());
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                res.fetch_mut::<Input>()
                    .set_button(button, state.is_pressed());
            }
            WindowEvent::CursorMoved { position, .. } => {
                res.fetch_mut::<Input>().cursor = glam::vec2(position.x as f32, position.y as f32);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(x, y) => glam::vec2(x, y),
                    // Touchpads report pixels; ~20 px per notch.
                    MouseScrollDelta::PixelDelta(p) => glam::vec2(p.x as f32, p.y as f32) / 20.0,
                };
                res.fetch_mut::<Input>().scroll += lines;
            }
            WindowEvent::Focused(false) => res.fetch_mut::<Input>().release_all(),
            WindowEvent::RedrawRequested => {
                self.app.update();
                self.apply_cursor();
                self.apply_settings();
                if self.app.exit_requested() {
                    event_loop.exit();
                } else if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // Raw motion, unaffected by cursor acceleration or window edges: right for mouse look.
        if let DeviceEvent::MouseMotion { delta } = event {
            self.app.resources.fetch_mut::<Input>().mouse_delta +=
                glam::vec2(delta.0 as f32, delta.1 as f32);
        }
    }
}
