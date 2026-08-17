//! Hosts a retained `wgpu_ui::Ui` for one shell panel, laid out into the body
//! rect the `Workspace` provides and drawn through the shared `MenuChrome` — the
//! non-modal analog of [`crate::uikit_overlay::Overlay`]. This is the harness
//! every panel uses: build the widget tree **once**, then each frame sync
//! editor→widget state, dispatch panel-scoped events, lay out into the rect, and
//! draw.
//!
//! # The recipe (stage U5)
//!
//! A panel is a **real widget tree** — stock `wgpu_ui` widgets in stock
//! containers — so it has no hit oracle, no panel-wide `ArmFire` and no `Hot`:
//! hover, arming and fire are each leaf's own, and a click comes back as an
//! **action tag** polled off `Ui::actions`. All ten docked panels are this now;
//! the pre-U5 "snapshot + faithful draw" pattern is gone, not deprecated.
//!
//! ## The shape
//!
//! ```text
//! root  = a thin composite widget holding the id tables
//!   └─ Linear::column(CrossAlign::Stretch)
//!        ├─ header    Length::Fit or Fixed   a Wrap (flows) or a Linear (does not)
//!        ├─ body      Length::Flex(1.0)      a ScrollArea XOR one content widget
//!        └─ footer    Length::Fixed          optional, e.g. an editor strip
//! ```
//!
//! - **`cross_align(CrossAlign::Stretch)` on the column is not optional.** A
//!   `Linear` measures to its *content*, so without it a `Linear` header is only
//!   as wide as its children and anything right-aligned in it stops early. (A
//!   `Wrap` header measures to the width available, which is why the tiles and
//!   templates columns never needed the line.)
//! - **A header's row height must be a *declared* constant.** Where the controls
//!   fit one row at every dock width that is `Length::Fixed(HEADER_H)`, and the
//!   body is its complement by construction. Where they flow, the *controls* must
//!   measure to the row and every run must be forced to it
//!   (`Wrap::run_extent`), or the band moves whenever the flow re-packs.
//! - **The body is a `ScrollArea` xor a content widget, never both.** A stock
//!   tree that scrolls goes in a `ScrollArea`; a domain surface that scrolls
//!   embeds its own `Scroller` and is arranged *into* its viewport. Two
//!   alternatives that must each fill the body go in a `Stack` of `Reveal`s — a
//!   hidden `Length::Flex` child hands its share to nobody.
//!
//! ## What a content widget may and may not own
//!
//! A content widget is the escape hatch for a **domain** surface: a cell grid, a
//! minimap, a colour-swatch field, a bitmask editor. It may own:
//!
//! - its **own geometry and hit oracle**, but only over its domain — *which cell
//!   is under this point*, which is not something a toolkit could know;
//! - an embedded `Scroller` (viewport + content height named in `arrange`), and
//!   the culling that goes with it;
//! - a **native GPU pass's rect**, reserved by `arrange` and read *back* off the
//!   widget after `build` — never recomputed by the shell, or the sprite and the
//!   well will disagree about the scroll offset;
//! - **stock children it arranges itself**, when the cell size is something no
//!   container measures to (`unitprops::ConnectorGrid`, `palette_panel::SwatchGrid`).
//!
//! It may **not** own: a button face, a toggle, a dropdown, a text field, a
//! scrollbar's paint, a hover derived from anything but `ctx.is_hovered`, or a
//! second keyboard path. Those are widgets; if one is missing, extend `wgpu-ui`.
//!
//! Two rules a content widget with children owes the `Ui`: expose them through
//! `child_count`/`child`/`child_mut` (or `descendant` cannot reach them and Tab
//! cannot skip them), and **override `hit_test`** — the default answers "me" for
//! the whole rect, which makes every child dead.
//!
//! ## Hosting rules the tree inherits
//!
//! - **Scroll is the widget's own, not snapshot state** (U2). Nothing about the
//!   offset lives on `EditorState`; a command that must move it leaves a one-shot
//!   request the shell hands over at sync, and the widget applies it at its next
//!   `arrange` — which is what clamps it against geometry that actually exists.
//! - **A text field's commit is the widget's, not a rect test** (U4). Poll
//!   `TextInput::take_commit()`: `Enter`, or `FocusOut` for *every* way focus
//!   leaves it, including presses this panel never receives. The shell's half is
//!   `blur_layer(layer, cause)` — `BlurCause::Moved` commits, `Cancelled`
//!   (Escape) abandons. A panel hosting a field must also check its **container's
//!   focus policy**: `ScrollArea::page_keys(PageKeys::WhenHovered)` is both the
//!   accelerator rule and a promise not to take the keyboard, and a container
//!   that grabs focus on a press blurs — and so *commits* — the field under it.
//! - **A dropdown is a hosted `wgpu_ui::Select`, not an open flag** (U3). The
//!   widget owns open/close/dismiss/pick/keyboard; the host arranges it, gives it
//!   first refusal, polls `take_pick()` and re-emits the pick as an action tag.
//!   Its list draws in the *overlay* pass, which [`PanelUi::build`] routes into
//!   the shell's popup layer — above every panel, and placed against the window
//!   rect [`PanelUi::set_viewport`] names, not the panel body.
//! - **One tag space, one poll.** Everything a panel produces comes back as an
//!   action tag off `Ui::actions`, decoded by the panel's own `action_of`. Kind 0
//!   is left unused so a stray zero resolves to nothing. The exceptions are
//!   outcomes that carry a **value no `u64` holds** — a text commit's `String`, a
//!   palette edit's colours — which get their own polled queue.
//! - **A `Select` and a `List` commit on the *press*, and `Ui::actions` lives for
//!   exactly one dispatch.** The shell must therefore drain such a panel after
//!   the **press** dispatch as well as after the release, in `Press::Body` beside
//!   `drain_palette`. A tree with no press-firing child needs no arm there —
//!   check the tree you actually built.
//! - **A header key whose precondition fails is disabled-dead, with the reason
//!   as its tooltip** (audit item 5, 2026-08-11). Each panel's `COMMANDS` table
//!   states the key's precondition once (a `Need`), and `sync` derives *both*
//!   the disabled state and the tooltip from it through [`sync_header_key`] —
//!   a disabled key still hit-tests and still reports its tooltip, which the
//!   shell mirrors into the status bar the moment hover lands. The greyed key
//!   is never the only guard: the command behind it still validates and fails
//!   loudly, because scripts, the console and keybindings reach it directly.
//!   (This supersedes G4's muted-but-live rule, which predates tooltips.)
//!
//! ## The `Snapshot`
//!
//! What survives of the pre-U5 recipe is the per-frame `Snapshot`: a borrow-free
//! copy of everything the panel reads from `EditorState`, so the retained tree
//! holds no borrow. It now feeds **`sync`**, which writes into retained children
//! (`set_text` / `set_selected` / `set_shown` reached by `descendant_mut`), not a
//! draw function that painted the panel from scratch. Two things follow:
//!
//! - **Re-sync, never rebuild.** A rebuilt subtree mints new ids, and hover,
//!   arming, capture, focus and a `TextInput`'s text all hang off the id. An
//!   optional row is a `wgpu_ui::Reveal` slot.
//! - **`sync` runs top-down.** A hidden `Reveal` reports *no children*, so it is
//!   out of `descendant` as well as out of Tab order: show the outer slot first,
//!   then reach inside it.
//!
//! ## References
//!
//! [`crate::savetools`] a stock tree (U5.2) · [`crate::minimap`] a header over a
//! content widget owning a native rect and a drag capture (U5.3) ·
//! [`crate::toolbox`] both, plus hosted dropdowns and the one-tag-space poll
//! (U5.4) · [`crate::templates_panel`] and [`crate::picker`] a flowed header over
//! a scrolling grid (U5.5/U5.6) · [`crate::units`] the simplest grid, a constant
//! band feeding three layers (U5.7) · [`crate::unitprops`] a **form** of `Reveal`
//! slots around retained `TextInput`s (U5.8) · [`crate::palette_panel`] two
//! panels in one tree, 256 retained swatches and value-carrying edit gestures
//! (U5.9). Per-ticket rationale is in `WGPU-UI-UNIFICATION.md` §6f.
//!
//! **`Hot` is gone** (U6.2). Nothing in the editor re-hit-tests a rect while
//! painting: a control's hover is its `Ui`'s, the panel frame's is the
//! `Workspace`'s own (`close_hovered`), and what is left of the shell's pointer
//! is `EditorState::cursor` — the **map's**, which is not a widget.

use wgpu_ui::{BlurCause, DrawList, DrawPass, Event, EventCtx, Response, Select, Ui, Widget, WidgetId, descendant_mut};

use crate::ui::Rect;
use crate::uikit_menu::MenuChrome;

/// The forwarding half of a thin panel root's `Widget` impl — the plumbing the
/// module doc's recipe makes identical across every docked panel, so each root
/// spells out only what is its own (`arrange` pre-passes, chrome under the
/// tree, drains in `event`).
///
/// Always generated: `measure` (a panel body fills what it is given), `rect`,
/// `id`, and the single-child contract — `child_count`/`child`/`child_mut`
/// plus the `hit_test` override (the default answers "me" for the whole body,
/// which would make every child dead; the tree hit-tests itself).
///
/// Optional arms, for the roots whose fn is a pure forward:
///
/// - `arrange` — store the rect, then measure *and* arrange the tree: a
///   `ScrollArea`/`Wrap` settles its content geometry in measure, and a host
///   that arranges without measuring first (the snapshot harness) must still
///   get a laid-out tree. A root that must resolve theme-measured widths
///   before that measure (a flowed header's keys, U7.1) writes its own
///   `arrange` and keeps the same measure-then-arrange tail.
/// - `draw` — forward in both passes (this root paints no chrome of its own),
///   so a child that grows a popup can reach the overlay pass (U3.2). A root
///   that paints a header band under the tree writes its own.
/// - `event` — forward untouched: nothing root-side to drain. A root hosting
///   a `Select` writes its own and drains through [`drain_selects`].
macro_rules! thin_root_plumbing {
	($($extra:ident),* $(,)?) => {
		fn measure(&mut self, avail: wgpu_ui::Size, _ctx: &mut wgpu_ui::LayoutCtx) -> wgpu_ui::Size {
			avail
		}

		fn rect(&self) -> wgpu_ui::Rect {
			self.rect
		}

		fn id(&self) -> wgpu_ui::WidgetId {
			self.id
		}

		fn child_count(&self) -> usize {
			1
		}

		fn child(&self, i: usize) -> Option<&dyn wgpu_ui::Widget> {
			(i == 0).then_some(&self.root as &dyn wgpu_ui::Widget)
		}

		fn child_mut(&mut self, i: usize) -> Option<&mut dyn wgpu_ui::Widget> {
			(i == 0).then_some(&mut self.root as &mut dyn wgpu_ui::Widget)
		}

		fn hit_test(&self, pos: wgpu_ui::Vec2) -> Option<wgpu_ui::WidgetId> {
			self.root.hit_test(pos)
		}

		$(crate::panel_ui::thin_root_plumbing!(@$extra);)*
	};
	(@arrange) => {
		fn arrange(&mut self, rect: wgpu_ui::Rect, ctx: &mut wgpu_ui::LayoutCtx) {
			self.rect = rect;
			self.root.measure(rect.size(), ctx);
			self.root.arrange(rect, ctx);
		}
	};
	(@draw) => {
		fn draw(&self, dl: &mut wgpu_ui::DrawList, ctx: &wgpu_ui::DrawCtx) {
			self.root.draw(dl, ctx);
		}
	};
	(@event) => {
		fn event(&mut self, ev: &wgpu_ui::Event, ctx: &mut wgpu_ui::EventCtx) -> bool {
			self.root.event(ev, ctx)
		}
	};
}
pub(crate) use thin_root_plumbing;

/// Drains every hosted `Select`'s pick and re-emits it as an action tag — the
/// U3 hosting rule shared by the five panels with dropdowns: a pick is the
/// dropdown's own commit, not a fire, so the root drains it in its `event`
/// (after forwarding to the tree) and fires the option's tag; the shell then
/// polls exactly one place for everything the panel produces. `tag_of` maps
/// (select index, picked option index) to that tag, or `None` to drop an
/// out-of-range pick.
pub fn drain_selects(
	root: &mut dyn Widget,
	ids: &[WidgetId],
	ctx: &mut EventCtx,
	mut tag_of: impl FnMut(usize, usize) -> Option<u64>,
) {
	for (i, &id) in ids.iter().enumerate() {
		let Some(sel) = descendant_mut::<Select>(root, id) else { continue };
		if let Some(picked) = sel.take_pick()
			&& let Some(tag) = tag_of(i, picked)
		{
			ctx.fire(id, Some(tag));
		}
	}
}

/// Applies the header-key convention to one command key: `unmet` names the
/// failed precondition (the key greys out dead and says why on hover), `None`
/// means the key is live and carries no tooltip. See the module doc's hosting
/// rules; the reason strings live in each panel's `Need` table.
pub fn sync_header_key(key: &mut wgpu_ui::Button, unmet: Option<&str>) {
	key.set_disabled(unmet.is_some());
	key.set_tooltip(unmet.map(str::to_string));
}

/// A retained widget tree bound to one panel.
pub struct PanelUi {
	pub ui: Ui,
}

impl PanelUi {
	pub fn new(root: impl Widget + 'static) -> Self {
		Self { ui: Ui::new(root) }
	}

	/// Names the surface this panel's **popups** must stay inside — the window,
	/// not the panel body. Set it before [`build`](Self::build); a dropdown near
	/// the panel's bottom edge then flips up at the *window's* edge, the way a
	/// dialog's does, instead of at its own panel's.
	pub fn set_viewport(&mut self, viewport: Rect) {
		self.ui.set_viewport(wgpu_ui::Rect::new(viewport.x, viewport.y, viewport.w, viewport.h));
	}

	/// Whether a dropdown/menu popup is open in this panel — which makes the
	/// panel **press-modal**: the next press anywhere on screen belongs to it
	/// (it picks an option or dismisses), exactly like an open menu cascade.
	/// The routing half rides on the pointer grab (the `Ui` reports an open
	/// popup as `Response::capturing`, so the router holds this layer); this
	/// accessor is the *render* half, what `over_at` and the hover suppression
	/// read.
	pub fn popup_open(&self) -> bool {
		self.ui.popup_open()
	}

	/// Dispatch `events` (panel-scoped; empty for view-only panels), lay the tree
	/// out into `body` (logical px), and append its draw commands to `dl`. Sync
	/// widget state via `self.ui.get_mut` *before* calling this; the caller
	/// composites `dl` through [`MenuChrome::render_list`]. Dispatch runs before
	/// layout so hit-testing uses the previous frame's geometry (the standard
	/// retained order, matching `Overlay::render`).
	///
	/// The two draw passes land in **two lists**: base chrome in `dl`, which the
	/// caller composites at this panel's own depth (clipped to its body), and the
	/// overlay pass — an open dropdown — in `popups`, which the shell accumulates
	/// across the whole panel loop and renders after it. A popup has to escape
	/// both the clip and the z-order, or the next panel paints over it.
	pub fn build(
		&mut self,
		chrome: &MenuChrome,
		body: Rect,
		scale: f32,
		events: &[Event],
		dl: &mut DrawList,
		popups: &mut DrawList,
	) {
		self.ui.set_scale(scale);
		if !events.is_empty() {
			self.ui.dispatch(events);
		}
		self.ui.layout_in(body, chrome.theme(), chrome.fonts());
		self.ui.draw_pass(dl, chrome.theme(), chrome.fonts(), DrawPass::Base);
		self.ui.draw_pass(popups, chrome.theme(), chrome.fonts(), DrawPass::Overlay);
	}

	/// Whether the panel's focused widget is a text field — the shell routes
	/// typing here (instead of the map bindings) and mirrors it into the OS IME.
	pub fn wants_text_input(&self) -> bool {
		self.ui.wants_text_input()
	}

	/// Whether *any* widget in the panel holds keyboard focus — a superset of
	/// [`wants_text_input`](Self::wants_text_input) (a focused list or dropdown
	/// wants keys but no typing). The router reads this after a press to decide
	/// which layer owns the keyboard.
	pub fn has_focus(&self) -> bool {
		self.ui.focused() != WidgetId::NONE
	}

	/// Give the keyboard back: no widget in this panel is focused any more. The
	/// shell blurs the outgoing layer when a press moves focus (`Moved`), and
	/// when Escape leaves a field (`Cancelled` — `TextInput` does not handle
	/// Escape itself). The **cause matters**: a hosted field commits its edit on
	/// a `Moved` blur and abandons it on a `Cancelled` one (U4.1).
	pub fn blur(&mut self, cause: BlurCause) {
		self.ui.blur(cause);
	}

	/// The focused text field's caret rect (logical px), anchoring the OS IME
	/// candidate window (`set_ime_cursor_area`). `None` when no field is focused.
	pub fn ime_rect(&self) -> Option<wgpu_ui::Rect> {
		self.ui.ime_rect()
	}

	/// Dispatch router-translated `events` into the panel's tree and write any
	/// copied/cut text out to the OS clipboard (the toolkit is clipboard-blind;
	/// the read half — Ctrl+V → `Event::Paste` — happens once, in
	/// [`UiRouter::translate`](crate::ui_router::UiRouter::translate)). Returns
	/// the toolkit's own verdict on the batch, which is what tells the shell
	/// whether to withhold the event from the layers under this one.
	pub fn dispatch_events(&mut self, events: &[Event]) -> Response {
		if events.is_empty() {
			return Response::default();
		}
		let response = self.ui.dispatch(events);
		if let Some(copied) = self.ui.take_clipboard() {
			crate::clipboard::set(&copied);
		}
		response
	}
}

/// The input half of a hosted panel, type-erased.
///
/// Every [`PanelHost<W>`] is a different type, so before U1.2 the shell carried
/// one `App::*_dispatch` helper per panel — eight of them, each synthesizing a
/// primary-only, `Modifiers::NONE`, move-less press. Behind this trait the shell
/// keeps **one** dispatch fn (`App::dispatch_layer`) keyed by [`Layer`], fed the
/// real translated events.
///
/// [`Layer`]: crate::ui_router::Layer
pub trait PanelInput {
	/// Dispatch router-translated `events` into this panel's tree, returning the
	/// toolkit's verdict: `wants_pointer` says whether the shell must withhold the
	/// event from the layers below (a press over empty panel chrome does not, and
	/// so falls through — minimap pan is the live example), and `capturing` says
	/// whether a widget holds the pointer — a drag in progress *or an open
	/// dropdown* — which is what the router tracks to keep feeding this layer
	/// once the cursor leaves it, and to route the dropdown's dismissing press
	/// here wherever it lands.
	fn dispatch(&mut self, events: &[Event]) -> Response;

	/// Drop this panel's keyboard focus, for `cause` — see [`PanelUi::blur`].
	/// (Reading focus needs no `&mut`, so the shell asks that through the
	/// shared-borrow `layer_panel` lookup instead.)
	fn blur(&mut self, cause: BlurCause);

	/// Name the surface this panel's popups must stay inside — see
	/// [`PanelUi::set_viewport`]. Set uniformly, for every hosted panel, so a
	/// panel that grows a dropdown later inherits the rule.
	fn set_viewport(&mut self, viewport: Rect);
}

impl<W: Widget + 'static> PanelInput for PanelHost<W> {
	fn dispatch(&mut self, events: &[Event]) -> Response {
		self.panel.dispatch_events(events)
	}

	fn blur(&mut self, cause: BlurCause) {
		self.panel.blur(cause);
	}

	fn set_viewport(&mut self, viewport: Rect) {
		self.panel.set_viewport(viewport);
	}
}

/// A [`PanelUi`] typed by its root content widget — the dispatch/outcome glue
/// every converted panel shares, so the shell keeps ONE generic impl instead
/// of a dispatch/outcome helper pair per panel.
pub struct PanelHost<W> {
	pub panel: PanelUi,
	id: WidgetId,
	_root: std::marker::PhantomData<W>,
}

impl<W: Widget + 'static> PanelHost<W> {
	pub fn new(root: W) -> Self {
		let id = root.id();
		Self { panel: PanelUi::new(root), id, _root: std::marker::PhantomData }
	}

	/// The typed root widget — sync per-frame state / poll its `take_outcome`.
	pub fn root_mut(&mut self) -> Option<&mut W> {
		self.panel.ui.get_mut::<W>(self.id)
	}

	/// The typed root widget, read-only — for state the shell mirrors rather
	/// than owns (a panel's own scroll offset, since U2), from a shared borrow.
	pub fn root(&self) -> Option<&W> {
		self.panel.ui.get::<W>(self.id)
	}

	/// Forwards to [`PanelUi::build`].
	pub fn build(
		&mut self,
		chrome: &MenuChrome,
		body: Rect,
		scale: f32,
		events: &[Event],
		dl: &mut DrawList,
		popups: &mut DrawList,
	) {
		self.panel.build(chrome, body, scale, events, dl, popups);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tabs::{TabAct, TabStrip};
	use std::path::Path;
	use wgpu_ui::{Modifiers, PointerButton, Vec2};

	/// The panel harness end to end, on the tab strip (a real converted panel):
	/// `build` dispatches the events it is given before laying out (a press +
	/// release through `build` fires the strip's outcome), and the type-erased
	/// [`PanelInput::dispatch`] reports consumption - a press over a tab is
	/// consumed, one over empty strip space falls through to the shell.
	#[test]
	fn build_dispatches_events_and_dispatch_reports_consumption() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");

		let button = |pressed: bool, x: f32, y: f32| Event::PointerButton {
			button: PointerButton::Primary,
			pressed,
			pos: Vec2::new(x, y),
			mods: Modifiers::NONE,
		};

		let mut host = PanelHost::new(TabStrip::new());
		let tabs = vec![("alpha".to_string(), false, false), ("beta".to_string(), false, false)];
		host.root_mut().expect("typed root resolvable").sync(tabs, 0, false);
		let body = Rect::new(0.0, 0.0, 400.0, 22.0);

		// First build: no events, just layout (hit-testing needs geometry).
		let mut dl = DrawList::new();
		host.build(&chrome, body, 1.0, &[], &mut dl, &mut DrawList::new());
		assert!(!dl.cmds.is_empty(), "the strip drew its tabs");

		// Second build: a press + release ride in as panel-scoped events and are
		// dispatched before layout - the strip's arm-fire completes.
		let mut dl = DrawList::new();
		host.build(
			&chrome,
			body,
			1.0,
			&[button(true, 10.0, 10.0), button(false, 10.0, 10.0)],
			&mut dl,
			&mut DrawList::new(),
		);
		assert_eq!(
			host.panel.ui.actions().iter().copied().find_map(crate::tabs::act_of),
			Some(TabAct::Select(0)),
			"build-fed events fired"
		);

		// The type-erased dispatch: consumed over tab 0, not over the empty right
		// end. Addressed as `&mut dyn PanelInput`, the way the router reaches
		// every panel through one dispatch fn.
		let host: &mut dyn PanelInput = &mut host;
		assert!(host.dispatch(&[button(true, 10.0, 10.0)]).wants_pointer(), "a press on a tab is consumed");
		assert!(host.dispatch(&[button(false, 10.0, 10.0)]).wants_pointer(), "the paired release completes the fire");
		assert!(!host.dispatch(&[]).wants_pointer(), "an empty batch is not a dispatch");
		assert!(
			!host.dispatch(&[button(true, 399.0, 10.0)]).wants_pointer(),
			"a press on empty strip space falls through"
		);
		// A stock key holds the pointer for the length of its own press - that is
		// how release-inside is decided when the pointer leaves the key - and lets
		// it go on the release, so nothing outlives the gesture (U1.3).
		assert!(!host.dispatch(&[button(false, 399.0, 10.0)]).capturing, "the capture ends with the release");
	}

	/// The D7 text-hosting wiring: a panel that hosts a `TextInput` reports
	/// `wants_text_input`/`ime_rect` only once the field is focused — the signals
	/// the shell uses to route keys and anchor the OS IME (the bug was that the
	/// Unit Properties panel never wired the IME at all).
	#[test]
	fn text_input_and_ime_state_track_focus() {
		let (device, queue, _serial) = crate::visual_test::test_gpu();
		let res = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources");
		let steel = crate::skin::load_steel(&res);
		let chrome = MenuChrome::new(&device, &queue, crate::capture::FORMAT, &steel).expect("chrome");

		let mut panel = PanelUi::new(wgpu_ui::TextInput::new());
		let body = Rect::new(0.0, 0.0, 200.0, 24.0);
		let mut dl = DrawList::new();
		panel.build(&chrome, body, 1.0, &[], &mut dl, &mut DrawList::new());

		// Unfocused: the shell must not steal typing or enable the IME.
		assert!(!panel.wants_text_input(), "an unfocused panel wants no text input");
		assert!(panel.ime_rect().is_none(), "no caret rect without focus");

		assert!(!panel.has_focus(), "and holds no keyboard focus at all");

		// Focus the hosted field, re-layout so it has caret geometry.
		panel.ui.focus_first();
		let mut dl = DrawList::new();
		panel.build(&chrome, body, 1.0, &[], &mut dl, &mut DrawList::new());
		assert!(panel.wants_text_input(), "a focused text field wants input");
		assert!(panel.ime_rect().is_some(), "a focused field exposes a caret rect for the IME");
		assert!(panel.has_focus(), "and the router sees this layer as the keyboard owner");

		// `blur` hands the keyboard back (U1.4): the router blurs the outgoing
		// layer when a press moves focus, and when Escape leaves a field - which
		// `TextInput` cannot do for itself, since it does not handle Escape.
		panel.blur(BlurCause::Moved);
		assert!(!panel.has_focus(), "blur gives the keyboard up");
		assert!(!panel.wants_text_input(), "so typing goes back to the shell's bindings");
		assert!(panel.ime_rect().is_none(), "and the OS IME is turned off with it");
	}
}
