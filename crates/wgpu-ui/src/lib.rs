//! `wgpu-ui` — a retained-mode GUI toolkit built on [`wgpu`].
//!
//! The crate is split into a **backend-agnostic core** and a **wgpu backend**:
//!
//! - The core ([`geom`], [`color`], [`draw`], widgets, layout, and event
//!   handling) is pure data and logic. Widgets resolve their geometry and emit
//!   a [`DrawList`]: an ordered display list of clipped, colored, optionally
//!   textured quads. No GPU is required to build or inspect one, so widget
//!   logic is unit-testable headless.
//! - The backend ([`gpu`]) turns a [`DrawList`] into draw calls. [`Renderer`]
//!   presents to a window surface; [`HeadlessRenderer`] renders offscreen and
//!   reads the pixels back — the harness used for automated visual tests.
//!
//! This mirrors the "one geometry, two passes" idea from the reference port:
//! draw in list order (painter's algorithm, z-order == order), hit-test in
//! reverse — but with retained widget objects that own their state.

pub mod color;
pub mod dock;
pub mod draw;
pub mod event;
pub mod geom;
pub mod gpu;
pub mod icon;
pub mod interact;
pub mod layout;
pub mod menu;
pub mod modal;
pub mod overlay;
pub mod scroll;
/// The masked secret-entry field (enable the `secret` feature).
#[cfg(feature = "secret")]
pub mod secret;
pub mod semantics;
pub mod split;
pub mod text;
pub mod textedit;
pub mod theme;
pub mod ui;
pub mod widget;
pub mod widgets;
pub mod window;
pub mod workspace;

/// winit event translation (enable the `winit` feature).
#[cfg(feature = "winit")]
pub mod winit;

/// Standalone application runner (enable the `winit` feature).
#[cfg(feature = "winit")]
pub mod app;

pub use color::Rgba;
pub use dock::{DockArea, DockLayout, Zone};
pub use draw::{DrawCmd, DrawList, IdleGate, TexRect, TextureId};
pub use event::{BlurCause, Event, Key, Modifiers, PointerButton, ScrollDelta};
pub use geom::{Insets, Rect, Size, Vec2};
pub use gpu::{FrameError, HeadlessRenderer, RenderError, Renderer, UiRenderer};
pub use icon::{Icon, Stencil};
pub use interact::{
    ArmFire, CommitPolicy, CursorIcon, IdGen, Response, WidgetId, WidgetState, next_id,
};
pub use layout::{Constrained, Fill, Grid, Linear, Reveal, Spacer, Stack, Wrap};
pub use menu::{ContextMenu, MenuBar, MenuItem};
pub use modal::Modal;
pub use overlay::{Select, SelectSize, Tabs, draw_select_box, draw_select_popup};
pub use scroll::{PageKeys, ScrollArea, Scroller};
#[cfg(feature = "secret")]
pub use secret::SecretInput;
pub use split::Split;
pub use text::{Font, FontError, FontId, Fonts, GlyphBitmap, PositionedGlyph};
pub use textedit::{Charset, TextAlign, TextArea, TextCommit, TextInput};
pub use theme::{
    Bevel, Emboss, Gunmetal, Metrics, POPUP_FRAME, ROW_FLOOR_ACTIVE, ROW_FLOOR_ACTIVE_HOVER,
    ROW_FLOOR_HOVER, Role, TextRole, Theme, emboss_offset,
};
pub use ui::{TabEntry, Ui, descendant, descendant_mut};
pub use widget::{
    Axis, CrossAlign, DrawCtx, DrawPass, EventCtx, LayoutCtx, Length, Limits, MainAlign, Semantics,
    Widget, kind_of,
};
pub use widgets::{
    Button, Checkbox, ColorButton, DragPhase, Image, ImageButton, Label, List, Panel, ProgressBar,
    Radio, RadioGroup, Separator, Slider, Toggle, Well,
};
pub use window::Window;
pub use workspace::{Workspace, WorkspaceError, WorkspaceLayout};

#[cfg(feature = "winit")]
pub use app::{App, AppConfig, Cx};

/// Re-export so consumers can name `wgpu` types without an explicit dependency
/// (and stay pinned to the exact version this crate was built against).
pub use wgpu;
