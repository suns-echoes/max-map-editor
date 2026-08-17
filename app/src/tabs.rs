//! Project tab strip: the row of open-project tabs below the menu bar. One
//! project is active at a time; click a tab to switch, click its `x` to close
//! (hidden when only one tab is open - the last stays). The shell (`main.rs`)
//! runs the resulting commands and the document model (`state.rs`) owns the
//! project list. Steel-themed like the rest of the chrome (active tab raised +
//! amber, others dim; dirty marked with `*`).
//!
//! It is a **widget tree**, not a painted band (U6.1): every tab is a stock
//! [`Button`] with a [`Label`] caption over it, and its close `x` a *frameless*
//! `Button` (`flat`) with its own glyph. There is no hit oracle, no panel-wide
//! `ArmFire` and no `Hot`: hover, arming and fire are each key's own, and a
//! click comes back as an action tag polled off `Ui::actions` ([`act_of`]).
//!
//! The strip owns only what no container can supply - the width rule
//! ([`tab_widths`], which compresses the row to fit the window) and the hit
//! order (`x` before body, since the `x` sits inside its tab) - and it arranges
//! its children itself, the shape `panel_ui.rs` sanctions for a widget whose
//! cell size is its own. The keys are **positional**: pool slot `i` is tab `i`,
//! whichever project that is after a close, which is exactly what the shell's
//! `Command::Tab { index }` means.

use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{Button, DrawList, Event, Label, Size, Vec2, WidgetId};

use crate::theme;
use crate::ui::Rect;
use crate::uikit_theme::rgba;

/// Tab strip height (px). Sits in `[menu::BAR_H, menu::BAR_H + BAR_H)`.
pub const BAR_H: f32 = 22.0;
const PAD: f32 = 8.0;
const CLOSE_W: f32 = 13.0;
const MIN_W: f32 = 70.0;
const MAX_W: f32 = 200.0;
const GAP: f32 = 2.0;
/// The floor a tab can compress to when the strip overflows the window -
/// keeps the close `x` and a few label glyphs usable.
const MIN_COMPRESSED: f32 = 44.0;

/// One tag space for the strip: `kind << 32 | payload`. Kind 0 is left unused,
/// so a stray zero resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// Select the tab whose index is the payload.
const KIND_SELECT: u64 = 1;
/// Close the tab whose index is the payload.
const KIND_CLOSE: u64 = 2;

const fn tag(kind: u64, payload: u64) -> u64 {
	(kind << KIND_SHIFT) | payload
}

/// What a fired tab key asks the shell for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabAct {
	Select(usize),
	Close(usize),
}

/// The strip action a fired tag stands for, or `None` if it is not one of this
/// strip's.
pub fn act_of(t: u64) -> Option<TabAct> {
	let payload = (t & 0xffff_ffff) as usize;
	match t >> KIND_SHIFT {
		KIND_SELECT => Some(TabAct::Select(payload)),
		KIND_CLOSE => Some(TabAct::Close(payload)),
		_ => None,
	}
}

/// A tab's label - dirty projects get a trailing `*` (title-bar parity); an
/// open save-editor session (a real `.DTA` save) is prefixed with a `/!\`
/// warning so a modified game save is unmistakable at a glance.
fn tab_label(name: &str, dirty: bool, saved: bool) -> String {
	let warn = if saved { "/!\\ " } else { "" };
	let star = if dirty { "*" } else { "" };
	format!("{warn}{name}{star}")
}

/// The ink a tab's caption reads in: an open save file always warns, an
/// ordinary map is amber while active and dim otherwise.
fn tab_ink(active: bool, saved: bool) -> [f32; 4] {
	if saved {
		theme::DEFECT
	} else if active {
		theme::ACCENT
	} else {
		theme::INK_DIM
	}
}

/// Per-tab widths: each fits its label (clamped MIN_W..MAX_W), then the whole
/// strip compresses equally toward [`MIN_COMPRESSED`] when it would overflow
/// the `vw`-wide window - labels ellipsize instead of tabs clipping off-screen.
/// `label_w` measures a composed label through the theme the strip is drawn
/// with, so the widths it hands the keys are the widths their captions need.
fn tab_widths(labels: &[String], vw: f32, label_w: impl Fn(&str) -> f32) -> Vec<f32> {
	let natural: Vec<f32> =
		labels.iter().map(|l| (PAD + label_w(l) + 6.0 + CLOSE_W + PAD).clamp(MIN_W, MAX_W)).collect();
	let gaps = (labels.len() as f32 + 1.0) * GAP;
	let total: f32 = natural.iter().sum();
	if total + gaps <= vw {
		return natural;
	}
	let scale = ((vw - gaps).max(0.0) / total.max(1.0)).min(1.0);
	natural.iter().map(|w| (w * scale).max(MIN_COMPRESSED)).collect()
}

/// The close-`x` hit area inside a tab rect.
fn close_rect(r: Rect) -> Rect {
	Rect::new(r.x + r.w - CLOSE_W - 4.0, r.y + (r.h - CLOSE_W) / 2.0, CLOSE_W, CLOSE_W)
}

/// A tab's close key: a frameless button (its only paint is the theme's
/// hover/press wash) under its own `x` glyph.
struct CloseKey {
	key: Button,
	glyph: Label,
}

impl CloseKey {
	fn new(i: usize) -> Self {
		Self {
			key: Button::new("").flat().action(tag(KIND_CLOSE, i as u64)),
			glyph: Label::new("x").small().raised().color(rgba(theme::CLOSE_INK)),
		}
	}
}

/// One tab: the face that selects it, the caption on that face, and - while the
/// strip is closable - the `x` inside it.
struct Tab {
	body: Button,
	caption: Label,
	close: Option<CloseKey>,
}

impl Tab {
	fn new(i: usize, closable: bool) -> Self {
		Self {
			body: Button::new("").action(tag(KIND_SELECT, i as u64)),
			caption: Label::new("").small().raised().ellipsize(),
			close: closable.then(|| CloseKey::new(i)),
		}
	}

	/// Widgets per tab - the divisor the flat child index runs on. Uniform
	/// across the strip, because `closable` is the strip's, not the tab's.
	fn width(closable: bool) -> usize {
		if closable { 4 } else { 2 }
	}

	fn child(&self, slot: usize) -> Option<&dyn Widget> {
		match slot {
			0 => Some(&self.body as &dyn Widget),
			1 => Some(&self.caption as &dyn Widget),
			2 => self.close.as_ref().map(|c| &c.key as &dyn Widget),
			3 => self.close.as_ref().map(|c| &c.glyph as &dyn Widget),
			_ => None,
		}
	}

	fn child_mut(&mut self, slot: usize) -> Option<&mut dyn Widget> {
		match slot {
			0 => Some(&mut self.body as &mut dyn Widget),
			1 => Some(&mut self.caption as &mut dyn Widget),
			2 => self.close.as_mut().map(|c| &mut c.key as &mut dyn Widget),
			3 => self.close.as_mut().map(|c| &mut c.glyph as &mut dyn Widget),
			_ => None,
		}
	}
}

/// The retained tab strip, hosted in a [`crate::panel_ui::PanelHost`] (the
/// steel band behind it is drawn shell-side, like the status bar's). Sync its
/// per-frame inputs with [`TabStrip::sync`]; poll what a click asked for off
/// the hosting `Ui`'s actions through [`act_of`].
pub struct TabStrip {
	id: WidgetId,
	tabs: Vec<Tab>,
	closable: bool,
	rect: Rect,
}

impl Default for TabStrip {
	fn default() -> Self {
		Self::new()
	}
}

impl TabStrip {
	pub fn new() -> Self {
		Self { id: wgpu_ui::next_id(), tabs: Vec::new(), closable: false, rect: Rect::ZERO }
	}

	/// Refresh the per-frame inputs: the open projects (`name`, `dirty`,
	/// `saved`), the active index, and whether the strip shows close buttons.
	///
	/// **Re-syncs, never rebuilds.** A project that stayed open keeps its key -
	/// and with it its hover and its arming; only a tab that appeared or
	/// disappeared moves the pool. Toggling `closable` *is* a shape change (the
	/// close keys come and go), so it rebuilds the pool, which is right: the last
	/// tab's `x` genuinely stops existing.
	pub fn sync(&mut self, tabs: Vec<(String, bool, bool)>, active: usize, closable: bool) {
		if self.closable != closable {
			self.closable = closable;
			self.tabs.clear();
		}
		self.tabs.truncate(tabs.len());
		while self.tabs.len() < tabs.len() {
			self.tabs.push(Tab::new(self.tabs.len(), closable));
		}
		for (i, (name, dirty, saved)) in tabs.iter().enumerate() {
			let t = &mut self.tabs[i];
			t.caption.set_text(tab_label(name, *dirty, *saved));
			t.caption.set_color(Some(rgba(tab_ink(i == active, *saved))));
			t.body.set_selected(i == active);
		}
	}

	/// Every widget in the strip, in draw order (a tab's face, then its caption,
	/// then its `x`), as one flat index space.
	fn nth(&self, i: usize) -> Option<&dyn Widget> {
		let w = Tab::width(self.closable);
		self.tabs.get(i / w).and_then(|t| t.child(i % w))
	}
}

impl Widget for TabStrip {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		Size::new(avail.w, BAR_H)
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		// Measured through the theme the strip draws with, so a caption's slot is
		// the width its own glyphs asked for. Resolved before the children are
		// arranged, since that needs `ctx` mutably.
		let labels: Vec<String> = self.tabs.iter().map(|t| t.caption.text().to_string()).collect();
		let widths = {
			let px = ctx.theme.font_px(wgpu_ui::TextRole::Small);
			let font = ctx.fonts.get(ctx.theme.font());
			tab_widths(&labels, rect.w, |s| font.measure(s, px))
		};
		// The caption stops clear of the `x` (which overlays the face's right end).
		let reserved = if self.closable { CLOSE_W + 4.0 } else { 0.0 };
		let mut x = rect.x + GAP;
		for (t, w) in self.tabs.iter_mut().zip(widths) {
			let r = Rect::new(x, rect.y, w, BAR_H - 1.0);
			t.body.arrange(r, ctx);
			t.caption.arrange(Rect::new(r.x + PAD, r.y, (r.w - PAD - reserved).max(0.0), r.h), ctx);
			if let Some(c) = &mut t.close {
				let cr = close_rect(r);
				c.key.arrange(cr, ctx);
				c.glyph.arrange(Rect::new(cr.x + 3.0, cr.y, cr.w - 3.0, cr.h), ctx);
			}
			x += w + GAP;
		}
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		// Chrome: base pass only (the band is drawn shell-side before this).
		if !ctx.is_base() {
			return;
		}
		for t in &self.tabs {
			t.body.draw(dl, ctx);
			t.caption.draw(dl, ctx);
			if let Some(c) = &t.close {
				c.key.draw(dl, ctx);
				c.glyph.draw(dl, ctx);
			}
		}
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		// Each key answers for itself; `hit_test` already decided which one the
		// pointer is on, so the `x` and the face beneath it never both arm.
		let mut handled = false;
		for t in &mut self.tabs {
			handled |= t.body.event(ev, ctx);
			if let Some(c) = &mut t.close {
				handled |= c.key.event(ev, ctx);
			}
		}
		handled
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	fn child_count(&self) -> usize {
		self.tabs.len() * Tab::width(self.closable)
	}

	fn child(&self, i: usize) -> Option<&dyn Widget> {
		self.nth(i)
	}

	fn child_mut(&mut self, i: usize) -> Option<&mut dyn Widget> {
		let w = Tab::width(self.closable);
		self.tabs.get_mut(i / w).and_then(|t| t.child_mut(i % w))
	}

	/// A tab's `x` sits **inside** its face, so it is asked first; the gaps
	/// between tabs answer nothing, which is what lets a press on empty strip
	/// space fall through to the shell.
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		if !self.rect.contains(pos) {
			return None;
		}
		self.tabs.iter().find_map(|t| {
			t.close
				.as_ref()
				.and_then(|c| c.key.hit_test(pos))
				.or_else(|| t.body.rect().contains(pos).then(|| t.body.id()))
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use wgpu_ui::event::{Modifiers, PointerButton};
	use wgpu_ui::{DrawCmd, Fonts, Ui};

	fn tabs() -> Vec<(String, bool, bool)> {
		vec![("alpha".into(), false, false), ("beta".into(), true, false), ("gamma".into(), false, false)]
	}

	/// The real chrome font + steel theme, with no GPU behind them - enough to
	/// measure, arrange and dispatch exactly as the shell does.
	fn fixture() -> (Fonts, crate::uikit_theme::SteelTheme) {
		let mut fonts = Fonts::new();
		let font =
			fonts.add(include_bytes!("../assets/MAX_Redesign_Square.ttf").to_vec()).expect("parse MAX_Redesign_Square");
		let em = fonts.get(font).units_per_em();
		let skin = crate::uikit_theme::SteelTheme::new(font, wgpu_ui::TextureId::ATLAS, em);
		(fonts, skin)
	}

	/// A laid-out strip in a `Ui`, ready to dispatch into, with the id to read it
	/// back by.
	fn laid_out(tabs: Vec<(String, bool, bool)>, active: usize, closable: bool, top: f32, vw: f32) -> (Ui, WidgetId) {
		let (fonts, skin) = fixture();
		let mut strip = TabStrip::new();
		strip.sync(tabs, active, closable);
		let id = strip.id();
		let mut ui = Ui::new(strip);
		ui.layout_in(Rect::new(0.0, top, vw, BAR_H), &skin, &fonts);
		(ui, id)
	}

	fn press(x: f32, y: f32) -> Event {
		Event::PointerButton {
			button: PointerButton::Primary,
			pressed: true,
			pos: Vec2::new(x, y),
			mods: Modifiers::NONE,
		}
	}

	fn release(x: f32, y: f32) -> Event {
		Event::PointerButton {
			button: PointerButton::Primary,
			pressed: false,
			pos: Vec2::new(x, y),
			mods: Modifiers::NONE,
		}
	}

	/// Where tab `i`'s face and `x` landed, read back off the arranged children -
	/// there is no geometry oracle to ask any more.
	fn rects(ui: &Ui, id: WidgetId, i: usize) -> (Rect, Option<Rect>) {
		let t = &ui.get::<TabStrip>(id).expect("the strip is the root").tabs[i];
		(t.body.rect(), t.close.as_ref().map(|c| c.key.rect()))
	}

	#[test]
	fn a_tab_body_selects_and_its_x_closes() {
		let (top, vw) = (24.0, 1280.0);
		let (mut ui, id) = laid_out(tabs(), 0, true, top, vw);
		let (body, close) = rects(&ui, id, 1);
		let close = close.expect("a closable strip has an x");

		ui.dispatch(&[press(body.x + 4.0, top + 4.0)]);
		assert!(ui.actions().is_empty(), "a key fires on the release, not the press");
		ui.dispatch(&[release(body.x + 4.0, top + 4.0)]);
		assert_eq!(ui.actions().iter().copied().find_map(act_of), Some(TabAct::Select(1)));

		ui.dispatch(&[press(close.x + 2.0, close.y + 2.0)]);
		ui.dispatch(&[release(close.x + 2.0, close.y + 2.0)]);
		assert_eq!(
			ui.actions().iter().copied().find_map(act_of),
			Some(TabAct::Close(1)),
			"the x is asked before the face it sits on"
		);

		// Pressed on a tab, released off it: nothing fires.
		ui.dispatch(&[press(body.x + 4.0, top + 4.0)]);
		ui.dispatch(&[release(vw - 1.0, top + 4.0)]);
		assert_eq!(ui.actions().iter().copied().find_map(act_of), None);
	}

	/// The lone blank scratch tab has no `x` at all - not an empty rect, no
	/// child - so its right-hand corner simply selects it.
	#[test]
	fn a_non_closable_strip_has_no_close_child() {
		let one = vec![("empty".to_string(), false, false)];
		let (top, vw) = (24.0, 1280.0);
		let (mut ui, id) = laid_out(one, 0, false, top, vw);
		let (body, close) = rects(&ui, id, 0);
		assert!(close.is_none(), "no close key exists to be hit");
		let c = close_rect(body);
		ui.dispatch(&[press(c.x + 2.0, c.y + 2.0)]);
		ui.dispatch(&[release(c.x + 2.0, c.y + 2.0)]);
		assert_eq!(ui.actions().iter().copied().find_map(act_of), Some(TabAct::Select(0)));
	}

	/// A press between two tabs (or past the last) belongs to nobody, so it falls
	/// through to the shell instead of being swallowed by the strip.
	#[test]
	fn the_gaps_between_tabs_answer_nothing() {
		let (top, vw) = (24.0, 1280.0);
		let (ui, id) = laid_out(tabs(), 0, true, top, vw);
		let strip = ui.get::<TabStrip>(id).expect("root");
		let (first, _) = rects(&ui, id, 0);
		assert_eq!(strip.hit_test(Vec2::new(first.right() + 1.0, top + 4.0)), None, "the gap after a tab");
		assert_eq!(strip.hit_test(Vec2::new(vw - 1.0, top + 4.0)), None, "the empty end of the strip");
		assert_eq!(strip.hit_test(Vec2::new(first.x + 2.0, top - 1.0)), None, "above the strip");
	}

	/// Closing a tab shrinks the pool from the end and re-labels what is left:
	/// the surviving keys keep their ids, so hover and arming survive a close.
	#[test]
	fn a_sync_relabels_in_place_instead_of_rebuilding() {
		let mut strip = TabStrip::new();
		strip.sync(tabs(), 0, true);
		let ids: Vec<WidgetId> = strip.tabs.iter().map(|t| t.body.id()).collect();
		strip.sync(tabs(), 2, true);
		assert_eq!(strip.tabs.iter().map(|t| t.body.id()).collect::<Vec<_>>(), ids, "a plain re-sync keeps every key");
		assert!(strip.tabs[2].body.selected() && !strip.tabs[0].body.selected(), "and moves the lit face");

		// Close the middle project: two tabs remain, holding the first two keys.
		let left = vec![tabs()[0].clone(), tabs()[2].clone()];
		strip.sync(left, 1, true);
		assert_eq!(strip.tabs.len(), 2);
		assert_eq!(strip.tabs.iter().map(|t| t.body.id()).collect::<Vec<_>>(), ids[..2], "no rebuild");
		assert_eq!(strip.tabs[1].caption.text(), "gamma", "the surviving project moved down a slot");
	}

	#[test]
	fn save_file_tab_gets_the_warning_prefix() {
		// A save-editor session is flagged with a leading `/!\`; the `*` still
		// trails when it is dirty. An ordinary map keeps its plain label.
		assert_eq!(tab_label("SAVE7.DAT", false, true), "/!\\ SAVE7.DAT");
		assert_eq!(tab_label("SAVE7.DAT", true, true), "/!\\ SAVE7.DAT*");
		assert_eq!(tab_label("mars.wrl", true, false), "mars.wrl*");
		assert_eq!(tab_label("mars.wrl", false, false), "mars.wrl");
	}

	/// The three inks a caption reads in, and that they reach the label.
	#[test]
	fn a_captions_ink_says_what_the_tab_is() {
		let mut strip = TabStrip::new();
		strip.sync(
			vec![("alpha".into(), false, false), ("beta".into(), false, false), ("SAVE7.DAT".into(), false, true)],
			1,
			true,
		);
		let ink = |i: usize| strip.tabs[i].caption.ink();
		assert_eq!(ink(0), Some(rgba(theme::INK_DIM)), "an inactive map is dim");
		assert_eq!(ink(1), Some(rgba(theme::ACCENT)), "the active one is amber");
		assert_eq!(ink(2), Some(rgba(theme::DEFECT)), "an open save file always warns");
	}

	/// Twelve long-named tabs in a narrow window: every tab stays on-screen
	/// (compressed equally), stays wide enough to use, and hit-tests where it
	/// was drawn.
	#[test]
	fn many_tabs_compress_to_fit_the_window() {
		let many: Vec<(String, bool, bool)> =
			(0..12).map(|i| (format!("a-rather-long-project-name-{i}.json"), i % 2 == 0, false)).collect();
		let (top, vw) = (24.0, 800.0);
		let (ui, id) = laid_out(many.clone(), 0, true, top, vw);
		let strip = ui.get::<TabStrip>(id).expect("root");
		for i in 0..many.len() {
			let (r, _) = rects(&ui, id, i);
			assert!(r.x + r.w <= vw + 0.5, "tab {i} overflows: ends at {}", r.x + r.w);
			assert!(r.w >= MIN_COMPRESSED - 0.5, "tab {i} unusably narrow: {}", r.w);
			assert_eq!(strip.hit_test(Vec2::new(r.x + 2.0, top + 4.0)), Some(strip.tabs[i].body.id()));
		}
		// A roomy window keeps natural widths (no needless compression).
		let (roomy, roomy_id) = laid_out(many, 0, true, top, 10_000.0);
		let (r, _) = rects(&roomy, roomy_id, 0);
		assert!(r.w > MIN_COMPRESSED + 10.0);
	}

	/// The close `x` is a frameless key: it paints nothing at rest, and the
	/// theme's wash - and only that - when the pointer is on it. (The face
	/// beneath it is the tab's own, and must not move.)
	#[test]
	fn the_close_x_washes_on_hover() {
		let (top, vw) = (24.0, 1280.0);
		let (fonts, skin) = fixture();
		let (mut ui, id) = laid_out(tabs(), 0, true, top, vw);
		let close = rects(&ui, id, 1).1.expect("closable");

		let washes = |ui: &Ui| -> Vec<wgpu_ui::Rgba> {
			let mut dl = DrawList::new();
			ui.draw(&mut dl, &skin, &fonts);
			dl.cmds
				.iter()
				.filter_map(|c| match c {
					DrawCmd::Solid { rect, color } if *rect == close => Some(*color),
					_ => None,
				})
				.collect()
		};
		assert!(washes(&ui).is_empty(), "no wash without a pointer on it");
		ui.dispatch(&[Event::PointerMoved { pos: Vec2::new(close.x + 2.0, close.y + 2.0) }]);
		assert_eq!(washes(&ui), vec![rgba(theme::HOVER)], "hover wash");
		ui.dispatch(&[press(close.x + 2.0, close.y + 2.0)]);
		assert_eq!(washes(&ui), vec![rgba(theme::PRESS)], "press wash");
	}

	/// Kind 0 is unused, so a stray zero resolves to nothing.
	#[test]
	fn tags_round_trip_to_actions() {
		assert_eq!(act_of(tag(KIND_SELECT, 3)), Some(TabAct::Select(3)));
		assert_eq!(act_of(tag(KIND_CLOSE, 3)), Some(TabAct::Close(3)));
		assert_eq!(act_of(0), None);
	}

	/// Every tab is reachable through the flat child index the `Ui` walks - and
	/// a non-closable strip is two widgets per tab, not four with holes.
	#[test]
	fn the_child_index_covers_every_key() {
		let mut strip = TabStrip::new();
		strip.sync(tabs(), 0, true);
		assert_eq!(strip.child_count(), 12);
		assert!((0..12).all(|i| Widget::child(&strip, i).is_some()));
		assert!(Widget::child(&strip, 12).is_none());
		strip.sync(tabs(), 0, false);
		assert_eq!(strip.child_count(), 6, "no close keys, no slots for them");
		assert!((0..6).all(|i| Widget::child(&strip, i).is_some()));
	}
}
