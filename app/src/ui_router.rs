//! The shell's **one** winit→toolkit event translator (stage U1 of
//! `WGPU-UI-UNIFICATION.md`, ticket U1.1).
//!
//! Every UI host used to own its own [`WinitInput`] — one per [`PanelUi`], one on
//! [`Overlay`], ~12 in all — each tracking cursor, modifiers, and scale
//! independently, and each text-hosting one re-implementing the Ctrl+V clipboard
//! bridge. Divergent modifier state was the concrete bug: a panel's translator
//! only ever saw `KeyboardInput`/`Ime` (never `ModifiersChanged`), so every event
//! it produced was stamped `Modifiers::NONE` and the Ctrl chords — paste, copy,
//! cut, select-all — were dead inside panel text fields while working in modals.
//!
//! Now there is exactly one translator, and it is the only thing in the crate
//! that touches a `WindowEvent`: hosts take pre-translated events
//! ([`Overlay::dispatch_events`], [`PanelUi::dispatch_events`]).
//!
//! Ticket U1.2 added the other half: [`Layer`], the shell's z-order written down
//! as an enum, and the hover retargeting that goes with it. Every host is now
//! addressed the same way — `App::dispatch_layer(layer, &events)` — so the eight
//! `App::*_dispatch` helpers, and with them the last synthesized events (a
//! primary-only, `Modifiers::NONE`, move-less press), are gone.
//!
//! Later U1 tickets grew this further: capture arbitration (U1.3), focus
//! arbitration (U1.4), and the map demoted from an implicit fallthrough to a
//! real answer of one hit test ([`Over`], U1.5).
//!
//! [`WinitInput`]: wgpu_ui::winit::WinitInput
//! [`PanelUi`]: crate::panel_ui::PanelUi
//! [`Overlay`]: crate::uikit_overlay::Overlay
//! [`Overlay::dispatch_events`]: crate::uikit_overlay::Overlay::dispatch_events
//! [`PanelUi::dispatch_events`]: crate::panel_ui::PanelUi::dispatch_events

use wgpu_ui::winit::WinitInput;
use wgpu_ui::{Event, Key};
use winit::event::WindowEvent;

/// The shell's UI layers in z-order, topmost first — the addresses a translated
/// event is dispatched to.
///
/// `window_event` always ran this cascade; it just ran it as a chain of ad-hoc
/// guards and one dispatch helper per panel. U1.2 writes the order down and
/// gives it a single dispatch fn (`App::dispatch_layer`). The map is not a
/// variant here because nothing is ever *dispatched* to it — it is a domain, not
/// a hosted `Ui`; it appears as [`Over::Map`], the bottom of the hit test.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layer {
	/// A `wgpu-ui` dialog — modal (swallows everything) or a non-blocking float
	/// (Fix Shore) that shares input with the live map.
	Overlay,
	/// The open right-click context menu.
	ContextMenu,
	/// The main menu bar and its open cascade.
	MenuBar,
	/// The project tab strip.
	Tabs,
	/// A docked panel, addressed by its workspace id (the `&'static str`
	/// `Workspace::body_at` resolves to).
	Panel(&'static str),
	/// The console — its hosted input line plus, since U4.5, the whole plate's
	/// pointer path: a press over the open band routes here (click-to-caret,
	/// drag-select), never to the menu bar or tab strip underneath.
	Console,
}

impl Layer {
	/// Every layer with a hosted `Ui`, for the two shell-wide broadcasts no
	/// single target owns: the pointer leaving the window and window focus loss.
	/// Both make *every* layer's hover / armed state stale, not just the
	/// topmost's. [`Layer::Overlay`] is absent — it buffers its own events and
	/// drains them at render time.
	pub const HOSTED: [Layer; 15] = [
		Layer::ContextMenu,
		Layer::MenuBar,
		Layer::Tabs,
		Layer::Console,
		Layer::Panel("tiles"),
		Layer::Panel("units"),
		Layer::Panel("minimap"),
		Layer::Panel("toolbox"),
		Layer::Panel("savetools"),
		Layer::Panel("passtools"),
		Layer::Panel("unitprops"),
		Layer::Panel("templates"),
		Layer::Panel("scenery"),
		Layer::Panel("palette"),
		Layer::Panel("wrlpalette"),
	];
}

/// What the pointer is over, resolved top-down through the z-order above — the
/// answer to the shell's **one** pointer hit test (`over_at`, ticket U1.5).
///
/// Before U1.5 that question was asked seven times, by seven hand-written
/// conjunctions of `lcy < BAR_H + tabs::BAR_H`, `Workspace::over_ui`,
/// `context_menu.is_some()` and `menu_ref().is_open()` — two in the pointer path
/// and five in the render path (the brush outline, the stamp ghost, the tile
/// ghost, the unit ghost, the status-bar cell readout). No two of them agreed:
/// four ignored an open menu cascade, one ignored the context menu. Each
/// disagreement is a place the map acts under UI that is covering it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
	/// A hosted UI layer — either the pointer is inside its rect, or it is one of
	/// the press-modal ones (an open menu cascade / context menu / dropdown),
	/// which take the next press wherever it lands.
	Ui(Layer),
	/// Workspace chrome with no hosted `Ui` of its own: splitters, dock edges,
	/// the reserved bottom strip. [`Workspace`] handles this input internally, so
	/// there is nothing to dispatch — but it is emphatically not the map.
	///
	/// [`Workspace`]: crate::workspace::Workspace
	Chrome,
	/// Nothing above it: the map — the fallthrough. The world layer the tools
	/// act on, and the only place they may.
	Map,
}

impl Over {
	/// Whether the pointer is over the bare map, i.e. no UI covers it. The map's
	/// one guard: paint/select/pan/zoom, the hover ghosts and the status-bar cell
	/// readout all ask exactly this.
	pub fn is_map(self) -> bool {
		self == Over::Map
	}
}

/// Translates window events into toolkit events for every UI host in the shell,
/// and tracks which [`Layer`] the pointer is over.
pub struct UiRouter {
	input: WinitInput,
	/// The layer the pointer was last over, so a move that changes the target can
	/// tell the layer it left. See [`retarget`](Self::retarget).
	hover: Option<Layer>,
	/// The layer holding the pointer mid-drag. See [`capture`](Self::capture).
	capture: Option<Layer>,
	/// The layer holding the keyboard. See [`focus`](Self::focus).
	focus: Option<Layer>,
}

impl UiRouter {
	/// `scale` is the editor's `ui_scale`; [`translate`](Self::translate) refreshes
	/// it per event, so this is only the value used before the first one.
	pub fn new(scale: f32) -> Self {
		Self { input: WinitInput::new(scale as f64), hover: None, capture: None, focus: None }
	}

	/// The layer that owns the keyboard, if one does — where keys, text, IME and
	/// paste go before the app's own bindings see them.
	///
	/// Focus follows the primary press: a press that focuses a widget takes the
	/// keyboard, a press anywhere else gives it up. Before U1.4 there was no such
	/// thing — keyboard reached exactly two places, each through its own bespoke
	/// gate (`unitprops_wants_text_input`, `console.is_open()`), so every other
	/// panel widget's keyboard behavior (`List` Up/Down, `Select` keys, Tab
	/// between fields) was unreachable by construction.
	pub fn focus(&self) -> Option<Layer> {
		self.focus
	}

	/// Move keyboard focus to `now`, returning the layer that must be **blurred**
	/// — its `Ui` still believes it holds the keyboard, and two trees each
	/// convinced of that is exactly the drift stage U1 exists to end. `None` when
	/// focus has not actually moved.
	pub fn refocus(&mut self, now: Option<Layer>) -> Option<Layer> {
		if self.focus == now {
			return None;
		}
		std::mem::replace(&mut self.focus, now)
	}

	/// The layer that owns the pointer stream right now, if one does.
	///
	/// A widget that starts a drag calls `ctx.capture(id)`, and its `Ui` reports
	/// that back as `Response.capturing`. Until U1.3 the shell dropped the signal,
	/// so a drag died the moment the cursor crossed out of the widget's rect —
	/// which is precisely why every panel drag had to be re-implemented shell-side
	/// (a scrollbar drag — retired in U2 — plus the palette editor's slider and
	/// block-bar drags, retired in U5.9, and `minipan`).
	/// While this is `Some`, **every**
	/// pointer event goes to that layer and nothing else — no other layer, and not
	/// the map — so a drag survives leaving both the widget and the panel.
	pub fn capture(&self) -> Option<Layer> {
		self.capture
	}

	/// Record what `layer`'s dispatch reported. Taking capture is unconditional
	/// (the layer that just captured is by definition the one holding the
	/// pointer); *releasing* it only counts from the holder, so the release
	/// broadcast — which dispatches to every layer, most of them capturing
	/// nothing — cannot knock a live drag loose.
	pub fn set_capture(&mut self, layer: Layer, capturing: bool) {
		if capturing {
			self.capture = Some(layer);
		} else if self.capture == Some(layer) {
			self.capture = None;
		}
	}

	/// Point the pointer at `now`, returning the layer it just *left* — which the
	/// caller must send [`Event::PointerLeft`], or that layer's hover stays lit
	/// after the cursor has moved on. `None` when the target is unchanged, which
	/// is every move but the one that crosses a boundary.
	pub fn retarget(&mut self, now: Option<Layer>) -> Option<Layer> {
		if self.hover == now {
			return None;
		}
		std::mem::replace(&mut self.hover, now)
	}

	/// Forget the hovered layer (the pointer left the window entirely), so
	/// re-entering the same panel counts as a fresh enter.
	pub fn clear_hover(&mut self) {
		self.hover = None;
	}

	/// Translate one window event at `scale` (the editor's `ui_scale`, mapping
	/// physical px → logical UI px) and bridge the OS clipboard.
	///
	/// **Call this exactly once per `WindowEvent`** — `window_event` does, at the
	/// top, and every host below borrows that one `Vec`. A second translation of
	/// the same event would re-read the OS clipboard on a Ctrl+V chord (and hand
	/// two hosts two `Event::Paste`s from one keystroke).
	///
	/// Events no host consumes still have to come through here: `ModifiersChanged`
	/// translates to nothing, yet it is what stamps every later pointer and key
	/// event — a host fed from a translator that never saw it (every panel, before
	/// U1.1) sees `Modifiers::NONE` forever and its Ctrl chords are dead.
	///
	/// The clipboard bridge: the toolkit is
	/// clipboard-blind, so a Ctrl+V chord is followed by an [`Event::Paste`]
	/// carrying the clipboard text. Scanning the just-translated events for the
	/// chord gets the modifier state for free.
	///
	/// The copy-*out* half of the bridge stays with each host (`Ui::take_clipboard`
	/// is per-`Ui`); it collapses in here once the router owns the layer stack.
	pub fn translate(&mut self, ev: &WindowEvent, scale: f32) -> Vec<Event> {
		self.input.set_scale(scale as f64);
		let mut events = Vec::new();
		self.input.handle(ev, &mut events);
		if paste_chord(&events)
			&& let Some(text) = crate::clipboard::get()
		{
			events.push(Event::Paste(text));
		}
		events
	}
}

/// Whether `events` contain a Ctrl+V press. `TextInput` handles Ctrl+C/X itself
/// (the text lands in `Ui::take_clipboard`), but it cannot *read* the OS
/// clipboard — so this is the one place the shell detects the paste chord.
fn paste_chord(events: &[Event]) -> bool {
	events
		.iter()
		.any(|e| matches!(e, Event::Key { key: Key::Character('v' | 'V'), pressed: true, mods, .. } if mods.ctrl))
}

#[cfg(test)]
mod tests {
	use super::*;
	use wgpu_ui::{Modifiers, PointerButton, Vec2};
	use winit::dpi::PhysicalPosition;
	use winit::event::{DeviceId, ElementState, MouseButton};
	use winit::keyboard::ModifiersState;

	fn key(c: char, pressed: bool, ctrl: bool) -> Event {
		Event::Key { key: Key::Character(c), pressed, repeat: false, mods: Modifiers { ctrl, ..Modifiers::NONE } }
	}

	/// The paste chord — the shell's single trigger for reading the OS clipboard.
	/// Either case of V with Ctrl held, on press only; a bare V, a release, and
	/// another Ctrl chord are not it.
	#[test]
	fn paste_chord_is_ctrl_v_press_either_case() {
		assert!(paste_chord(&[key('v', true, true)]), "Ctrl+v");
		assert!(paste_chord(&[key('V', true, true)]), "Ctrl+V (shifted / caps)");
		assert!(paste_chord(&[Event::PointerLeft, key('v', true, true)]), "found among other events");

		assert!(!paste_chord(&[]), "no events");
		assert!(!paste_chord(&[key('v', true, false)]), "a bare v is typing, not pasting");
		assert!(!paste_chord(&[key('v', false, true)]), "the release must not paste a second time");
		assert!(!paste_chord(&[key('c', true, true)]), "Ctrl+C is the toolkit's own copy");
	}

	/// Why there is only one translator: `ModifiersChanged` produces no event of
	/// its own, but it is what stamps every later pointer/key event — so a host
	/// fed from a translator that never sees it (every panel, before U1.1) sees
	/// `Modifiers::NONE` forever and its Ctrl chords are dead.
	#[test]
	fn tracked_modifiers_stamp_later_events() {
		let mut router = UiRouter::new(1.0);
		let ctrl = ModifiersState::CONTROL;
		assert!(
			router.translate(&WindowEvent::ModifiersChanged(ctrl.into()), 1.0).is_empty(),
			"held modifiers reach no host on their own - they are pure tracked state"
		);

		let events = router.translate(
			&WindowEvent::MouseInput {
				device_id: DeviceId::dummy(),
				state: ElementState::Pressed,
				button: MouseButton::Left,
			},
			1.0,
		);
		assert_eq!(
			events,
			vec![Event::PointerButton {
				button: PointerButton::Primary,
				pressed: true,
				pos: Vec2::ZERO,
				mods: Modifiers { ctrl: true, ..Modifiers::NONE },
			}],
			"the held Ctrl reaches the host"
		);
	}

	/// Hover enter/leave: [`UiRouter::retarget`] names the layer the pointer just
	/// left — and only then. Without it a panel keeps the hover it had when the
	/// cursor wandered off it, because a `Ui` clears `hovered` on nothing but a
	/// move it can hit-test or an explicit `PointerLeft`.
	#[test]
	fn retarget_names_only_the_layer_actually_left() {
		let mut router = UiRouter::new(1.0);
		let (tiles, units) = (Layer::Panel("tiles"), Layer::Panel("units"));

		assert_eq!(router.retarget(Some(tiles)), None, "entering from the map leaves nothing behind");
		assert_eq!(router.retarget(Some(tiles)), None, "a move *within* one panel is not a retarget");
		assert_eq!(router.retarget(Some(units)), Some(tiles), "panel -> panel: the old one must be told");
		assert_eq!(router.retarget(None), Some(units), "panel -> the map: likewise");
		assert_eq!(router.retarget(None), None, "moving about the map retargets nothing");

		// Leaving the window forgets the target, so re-entering the *same* panel
		// is a fresh enter rather than a no-op retarget.
		router.retarget(Some(tiles));
		router.clear_hover();
		assert_eq!(router.retarget(Some(tiles)), None);
	}

	/// Capture arbitration: the layer that reports `capturing` holds the pointer
	/// stream until *it* lets go. The asymmetry is the point — anyone may take
	/// capture, but only the holder can drop it, so the release broadcast (which
	/// dispatches to every layer, nearly all of them capturing nothing) cannot
	/// knock a live drag loose.
	#[test]
	fn capture_is_held_until_its_owner_releases_it() {
		let mut router = UiRouter::new(1.0);
		let (unitprops, tiles) = (Layer::Panel("unitprops"), Layer::Panel("tiles"));
		assert_eq!(router.capture(), None, "nothing captures until a widget starts a drag");

		// A text field begins a drag-select.
		router.set_capture(unitprops, true);
		assert_eq!(router.capture(), Some(unitprops));

		// The cursor wanders over another panel and off the UI entirely: neither
		// steals the stream, and a `PointerLeft` to a *different* layer must not
		// end the drag either.
		router.set_capture(tiles, false);
		assert_eq!(router.capture(), Some(unitprops), "a non-owner's dispatch cannot release the capture");
		router.retarget(None);
		assert_eq!(router.capture(), Some(unitprops), "hover and capture are independent");

		// The real release: the owner stops reporting `capturing`.
		router.set_capture(unitprops, false);
		assert_eq!(router.capture(), None);
		router.set_capture(unitprops, false);
		assert_eq!(router.capture(), None, "releasing twice is harmless");
	}

	/// Focus arbitration: [`UiRouter::refocus`] names the layer that must be
	/// **blurred**, because its `Ui` still believes it holds the keyboard. Two
	/// trees each convinced of that is the drift stage U1 exists to end — and it
	/// is not hypothetical: focus is per-`Ui`, so nothing in the toolkit stops a
	/// second panel from focusing a field while the first still shows a caret.
	#[test]
	fn refocus_names_the_layer_that_must_be_blurred() {
		let mut router = UiRouter::new(1.0);
		let (unitprops, console) = (Layer::Panel("unitprops"), Layer::Console);
		assert_eq!(router.focus(), None, "nothing holds the keyboard until a press focuses something");

		assert_eq!(router.refocus(Some(unitprops)), None, "the first focus displaces nobody");
		assert_eq!(router.focus(), Some(unitprops));
		assert_eq!(router.refocus(Some(unitprops)), None, "re-focusing the same layer is not a move");

		assert_eq!(router.refocus(Some(console)), Some(unitprops), "the console takes it - blur the panel");
		assert_eq!(router.refocus(None), Some(console), "a press on the map gives the keyboard up");
		assert_eq!(router.refocus(None), None, "and again changes nothing");

		// Focus and capture are independent: a drag does not move the keyboard,
		// and losing the keyboard does not end a drag.
		router.refocus(Some(unitprops));
		router.set_capture(unitprops, true);
		router.refocus(None);
		assert_eq!(router.capture(), Some(unitprops), "blurring a layer does not drop its pointer capture");
	}

	/// The `scale` argument is applied per event (the editor's `ui_scale` can
	/// change between them), mapping physical px to the logical px hosts lay out
	/// in — and the tracked cursor is shared, so a later button press is stamped
	/// with the position from the move.
	#[test]
	fn scale_applies_per_event_and_the_cursor_is_shared() {
		let mut router = UiRouter::new(1.0);
		let moved =
			|x, y| WindowEvent::CursorMoved { device_id: DeviceId::dummy(), position: PhysicalPosition::new(x, y) };

		assert_eq!(
			router.translate(&moved(200.0, 100.0), 2.0),
			vec![Event::PointerMoved { pos: Vec2::new(100.0, 50.0) }],
			"physical / ui_scale"
		);
		assert_eq!(
			router.translate(&moved(200.0, 100.0), 1.0),
			vec![Event::PointerMoved { pos: Vec2::new(200.0, 100.0) }],
			"a scale change applies to the next event"
		);

		let press = router.translate(
			&WindowEvent::MouseInput {
				device_id: DeviceId::dummy(),
				state: ElementState::Pressed,
				button: MouseButton::Left,
			},
			1.0,
		);
		assert_eq!(
			press,
			vec![Event::PointerButton {
				button: PointerButton::Primary,
				pressed: true,
				pos: Vec2::new(200.0, 100.0),
				mods: Modifiers::NONE,
			}],
			"the press is stamped with the tracked cursor"
		);
	}
}
