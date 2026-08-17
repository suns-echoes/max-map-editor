//! A standalone application runner (enable the `winit` feature).
//!
//! [`run`] owns the winit window, the [`Renderer`], the [`Ui`], the
//! [`WinitInput`] translator, and the per-frame cycle — dispatch → your
//! [`App::update`] → layout → draw → present — and keeps the UI scale in lockstep
//! with the renderer (one source of truth). A consumer implements [`App`] with
//! three hooks (`setup`, `build`, `update`) instead of hand-writing an
//! `ApplicationHandler`. Apps that embed `wgpu-ui` into their own loop/renderer
//! (sharing a device with other passes) keep driving [`Ui`] directly and skip
//! this module.
//!
//! Scope, honestly: the runner suits demos and tools. It is single-window and
//! has **no IME** (composed CJK/dead-key input never reaches [`TextInput`]
//! (crate::TextInput)) — close those gaps in your own loop if you need them.
//! Native only: the whole crate assumes a blocking device (wasm would hang in
//! its internal `block_on`).

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::color::Rgba;
use crate::draw::{DrawList, IdleGate};
use crate::event::Event;
use crate::geom::{Size, Vec2};
use crate::gpu::{FrameError, Renderer};
use crate::interact::Response;
use crate::text::Fonts;
use crate::theme::Theme;
use crate::ui::Ui;
use crate::winit::WinitInput;

/// Window + loop configuration, returned by [`App::config`].
pub struct AppConfig {
    /// Window title.
    pub title: String,
    /// Initial inner size in logical pixels.
    pub size: (f64, f64),
    /// The clear color behind the UI.
    pub clear: Rgba,
    /// Redraw every frame (for animations) rather than only on input.
    pub continuous: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "wgpu-ui".to_string(),
            size: (960.0, 640.0),
            clear: Rgba::hex(0x101014),
            continuous: false,
        }
    }
}

/// Per-frame context handed to [`App::update`], after the UI has dispatched the
/// frame's events and before layout/draw.
pub struct Cx<'a> {
    /// The widget tree — read widget state and push host state in here.
    pub ui: &'a mut Ui,
    /// The renderer — register/update host textures here.
    pub renderer: &'a mut Renderer,
    /// The shared font set.
    pub fonts: &'a Fonts,
    /// This frame's translated input events (route what the UI didn't consume
    /// to your world).
    pub events: &'a [Event],
    /// What the UI consumed this frame (see [`Response::wants_pointer`]).
    pub response: Response,
    /// The current cursor position (logical px).
    pub cursor: Vec2,
    exit: &'a mut bool,
}

impl Cx<'_> {
    /// Requests application exit after this frame.
    pub fn exit(&mut self) {
        *self.exit = true;
    }
}

/// The application a [`run`] drives. Implement the three hooks; the runner owns
/// the window, renderer, input, and frame loop.
pub trait App {
    /// Window/loop configuration. Default: an untitled 960×640 window.
    fn config(&self) -> AppConfig {
        AppConfig::default()
    }

    /// Register fonts (via [`Fonts::add`]) and return the [`Theme`] to paint
    /// with. Called once before the window exists.
    fn setup(&mut self, fonts: &mut Fonts) -> Box<dyn Theme>;

    /// Build the widget tree. The [`Renderer`] is available so host textures can
    /// be registered up front (their [`TextureId`](crate::draw::TextureId)s go
    /// into the tree).
    fn build(&mut self, renderer: &mut Renderer, fonts: &Fonts) -> Ui;

    /// React to the dispatched frame: poll fired widgets / actions, push host
    /// state into widgets, route un-consumed input to your world. Default: nothing.
    fn update(&mut self, cx: &mut Cx) {
        let _ = cx;
    }
}

/// Runs `app` to completion (until the window closes or [`Cx::exit`]).
pub fn run(mut app: impl App + 'static) -> Result<(), Box<dyn std::error::Error>> {
    let config = app.config();
    let mut fonts = Fonts::new();
    let theme = app.setup(&mut fonts);
    let event_loop = EventLoop::new()?;
    let mut runner = Runner {
        app,
        fonts,
        theme,
        config,
        state: None,
    };
    event_loop.run_app(&mut runner)?;
    Ok(())
}

struct State {
    window: Arc<Window>,
    renderer: Renderer,
    ui: Ui,
    input: WinitInput,
    events: Vec<Event>,
    /// An identical frame is not re-presented (idle gating; see `redraw`).
    gate: IdleGate,
    /// The last OS cursor applied (only changes reach the window).
    last_cursor: crate::interact::CursorIcon,
    /// The last IME-enabled state / candidate-window anchor applied.
    last_ime: bool,
    last_ime_rect: crate::geom::Rect,
}

struct Runner<A: App> {
    app: A,
    fonts: Fonts,
    theme: Box<dyn Theme>,
    config: AppConfig,
    state: Option<State>,
}

impl<A: App> Runner<A> {
    fn redraw(&mut self, el: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        let events = std::mem::take(&mut state.events);
        // One scale source: the renderer's, mirrored into the Ui so layout/draw
        // and the renderer's upscale agree (and scale-aware widgets pixel-lock).
        let scale = state.renderer.scale();
        state.ui.set_scale(scale);

        let response = state.ui.dispatch(&events);
        // Apply the UI's requested mouse cursor (I-beam over text fields,
        // resize arrows over grips/splitters) — only on change.
        let icon = state.ui.cursor_icon();
        if icon != state.last_cursor {
            state.last_cursor = icon;
            state.window.set_cursor(crate::winit::map_cursor(icon));
        }
        // Mirror focus into the OS IME: enabled over text fields, the
        // candidate window anchored at the caret — only changes reach winit.
        let ime = state.ui.wants_text_input();
        if ime != state.last_ime {
            state.last_ime = ime;
            state.window.set_ime_allowed(ime);
        }
        if ime
            && let Some(r) = state.ui.ime_rect()
            && r != state.last_ime_rect
        {
            state.last_ime_rect = r;
            state.window.set_ime_cursor_area(
                LogicalPosition::new(r.x as f64, r.y as f64),
                LogicalSize::new(r.w as f64, r.h as f64),
            );
        }
        let cursor = state.input.cursor();
        let mut exit = false;
        {
            let mut cx = Cx {
                ui: &mut state.ui,
                renderer: &mut state.renderer,
                fonts: &self.fonts,
                events: &events,
                response,
                cursor,
                exit: &mut exit,
            };
            self.app.update(&mut cx);
        }
        if exit {
            el.exit();
            return;
        }

        // Idle gating, outer half: a clean tree (no events, no state pushed in
        // `update`) skips layout and draw entirely — an idle frame does no
        // work at all. Continuous mode animates, so it always draws.
        if !state.ui.take_dirty() && !self.config.continuous {
            return;
        }

        let (pw, ph) = state.renderer.size();
        let logical = Size::new(pw as f32 / scale, ph as f32 / scale);
        state.ui.layout(logical, self.theme.as_ref(), &self.fonts);
        let mut dl = DrawList::new();
        state.ui.draw(&mut dl, self.theme.as_ref(), &self.fonts);

        // Idle gating, inner half ([`IdleGate`]): events flowed but the pixels
        // came out identical — an identical draw list at an identical
        // size/scale is not re-presented. A desktop tool under a moving mouse
        // would otherwise render at full rate for nothing.
        if state.gate.changed(&dl, (pw, ph), scale) {
            match state.renderer.render(&self.fonts, &dl, self.config.clear) {
                Ok(()) => state.gate.accept(dl, (pw, ph), scale),
                // A timed-out or occluded frame is transient — a minimized
                // window must not take the app down with it. Try again next
                // redraw: the gate never saw an accept and the tree re-marks,
                // so neither half short-circuits the retry away.
                Err(FrameError::Timeout | FrameError::Occluded) => {
                    state.ui.mark_dirty();
                    state.window.request_redraw();
                }
                // Anything else (device loss, out of memory) would otherwise be
                // an invisible black window: say why and stop.
                Err(e) => {
                    eprintln!("wgpu-ui: render failed: {e}");
                    el.exit();
                    return;
                }
            }
        }

        if self.config.continuous {
            state.window.request_redraw();
        }
    }
}

impl<A: App> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(self.config.title.clone())
            .with_inner_size(LogicalSize::new(self.config.size.0, self.config.size.1));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("wgpu-ui: failed to create window: {e}");
                el.exit();
                return;
            }
        };
        let size = window.inner_size();
        let scale = window.scale_factor();
        let mut renderer =
            match Renderer::new(window.clone(), size.width.max(1), size.height.max(1)) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("wgpu-ui: failed to create renderer: {e}");
                    el.exit();
                    return;
                }
            };
        renderer.set_scale(scale as f32);
        let input = WinitInput::new(scale);
        let ui = self.app.build(&mut renderer, &self.fonts);
        self.state = Some(State {
            window,
            renderer,
            ui,
            input,
            events: Vec::new(),
            gate: IdleGate::new(),
            last_cursor: crate::interact::CursorIcon::Default,
            last_ime: false,
            last_ime_rect: crate::geom::Rect::ZERO,
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(state) = self.state.as_mut() {
            state.input.handle(&event, &mut state.events);
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = self.state.as_mut() {
                    state.renderer.resize(size.width.max(1), size.height.max(1));
                    // A resize changes pixels the tree can't see coming.
                    state.ui.mark_dirty();
                    state.window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(state) = self.state.as_mut() {
                    state.renderer.set_scale(scale_factor as f32);
                    state.input.set_scale(scale_factor);
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(el),
            // Only window events the translator turned into pending UI events
            // nudge a redraw — other chatter (Moved, Occluded, focus churn with
            // nothing pending) doesn't spin the frame loop.
            _ => {
                if let Some(state) = self.state.as_ref()
                    && !state.events.is_empty()
                {
                    state.window.request_redraw();
                }
            }
        }
    }
}

// What these tests can and cannot reach: only the pure configuration surface
// (`AppConfig::default` and the `App::config` default hook) runs without a
// display. Everything else is event-loop glue by construction — `run` needs
// `EventLoop::new()` (a display server), `Runner::{redraw, resumed,
// window_event}` take an `&ActiveEventLoop` that only exists inside a running
// loop, and `Cx` (so also `Cx::exit` and the `App::update` default) cannot be
// built without a `Renderer`, whose constructor requires a live window surface
// (`wgpu::SurfaceTarget`). None of that can be honestly exercised headlessly.
#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal [`App`] that overrides nothing optional; the hooks that
    /// cannot run without a window are left unreachable.
    struct Minimal;

    impl App for Minimal {
        fn setup(&mut self, _fonts: &mut Fonts) -> Box<dyn Theme> {
            unreachable!("setup needs no test: it is never called off-window here")
        }
        fn build(&mut self, _renderer: &mut Renderer, _fonts: &Fonts) -> Ui {
            unreachable!("build needs a Renderer, which needs a window surface")
        }
    }

    /// The documented default window/loop configuration: a "wgpu-ui"-titled
    /// 960x640 window, near-black clear, redrawing only on input.
    #[test]
    fn app_config_default_is_the_documented_window() {
        let c = AppConfig::default();
        assert_eq!(c.title, "wgpu-ui");
        assert_eq!(c.size, (960.0, 640.0), "initial inner size, logical px");
        assert_eq!(c.clear, Rgba::hex(0x101014), "clear color behind the UI");
        assert!(!c.continuous, "redraw only on input by default");
    }

    /// An `App` that doesn't override `config` gets `AppConfig::default()`
    /// from the trait's default hook (what `run` consults before the window
    /// exists).
    #[test]
    fn app_trait_default_config_matches_appconfig_default() {
        let c = Minimal.config();
        let d = AppConfig::default();
        assert_eq!(c.title, d.title, "default hook returns the default title");
        assert_eq!(c.size, d.size, "default hook returns the default size");
        assert_eq!(c.clear, d.clear, "default hook returns the default clear");
        assert_eq!(
            c.continuous, d.continuous,
            "default hook returns the default redraw policy"
        );
    }
}
