//! Platform layer: window, input and the main loop (winit).
//! Publishes [`SurfaceTarget`] for renderers and [`Input`] for game code.

mod input;

pub use input::Input;

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
use winit::window::{CursorGrabMode, Window, WindowId};

#[derive(Clone, Debug)]
pub struct WindowPlugin {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

impl Default for WindowPlugin {
    fn default() -> Self {
        Self {
            title: "Olvin".into(),
            width: 1280,
            height: 720,
            resizable: true,
        }
    }
}

impl Plugin for WindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Input>()
            .init_resource::<CursorOptions>()
            .add_system(Stage::Last, |res| res.fetch_mut::<Input>().end_frame());
        let config = self.clone();
        app.set_runner(move |app| run(app, config));
    }
}

fn run(app: App, config: WindowPlugin) {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut runner = Runner {
        app,
        config,
        window: None,
        cursor: CursorOptions::default(),
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
}

impl Runner {
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
        let attrs = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height))
            .with_resizable(self.config.resizable);
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                log::error!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.app.insert_resource(SurfaceTarget::new(
            window.clone(),
            (size.width, size.height),
        ));
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
