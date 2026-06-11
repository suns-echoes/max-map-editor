//! Color Palette dockable, laid out by the palette slot
//! contract (`docs/design/tileset-contract.md` §1): labeled sections (the
//! label ink + an amber tick mark the editable dynamic slots, 64–159),
//! animated classes dotted, duplicate colors in the dynamic range flagged.
//! Swatches are always opaque — they show the true palette colour.
//! All 256 slots are always visible (no scroll — the design's rule); the
//! grid is pure rects + labels on the UI quad layer, no GPU pass.
//!
//! Single click selects (`color N`); the editor strip below the grid edits
//! the selected **dynamic** slot in HSL — and for water cycle slots a
//! second bar row re-tints the whole animated block (`hsl-block`). Edits
//! land as project palette overrides (map-specific colors), undoable.

use map_core::{WATER_CYCLES, rgb_to_hsl};

use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};

const COLS: u16 = 8; // 8 swatches per line — a water-cycle block reads as one row
const PAD: f32 = 4.0;
const LABEL_H: f32 = 13.0;
const GAP: f32 = 1.0;
/// The header row (cycle/static buttons), pinned above the grid.
pub const HEADER_H: f32 = 22.0;
/// The editor strip at the panel bottom: info + RGB + HSL + block rows.
pub const EDITOR_H: f32 = 78.0;
const BAR_H: f32 = 13.0;
/// Block-bar drag sensitivity: degrees of hue per px; S/L fraction per px.
pub const HUE_PER_PX: f32 = 1.0;
pub const SL_PER_PX: f32 = 0.005;

/// One contract range (`end` inclusive).
pub struct Section {
	pub label: &'static str,
	pub start: u16,
	pub end: u16,
	pub editable: bool,
	pub animated: bool,
}

/// The palette slot contract, §1. Dynamic slots 64–159 belong to the
/// tileset; everything else is the game's. Animated = color-cycled in game
/// (9–31 system sparkle/sea, 96–127 the per-planet water colors). Each
/// water cycle block gets its own labeled line — one block = one gradient,
/// reading it as a row is the point.
pub const SECTIONS: [Section; 11] = [
	Section { label: "system 0-8", start: 0, end: 8, editable: false, animated: false },
	Section { label: "game animated 9-31", start: 9, end: 31, editable: false, animated: true },
	Section { label: "game ramps 32-63", start: 32, end: 63, editable: false, animated: false },
	Section { label: "map tiles 64-95", start: 64, end: 95, editable: true, animated: false },
	Section { label: "water cycle 96-102", start: 96, end: 102, editable: true, animated: true },
	Section { label: "water cycle 103-109", start: 103, end: 109, editable: true, animated: true },
	Section { label: "water cycle 110-116", start: 110, end: 116, editable: true, animated: true },
	Section { label: "water cycle 117-122", start: 117, end: 122, editable: true, animated: true },
	Section { label: "water cycle 123-127", start: 123, end: 127, editable: true, animated: true },
	Section { label: "map tiles 128-159", start: 128, end: 159, editable: true, animated: false },
	Section { label: "game ramps 160-255", start: 160, end: 255, editable: false, animated: false },
];

/// Is a slot tileset-editable (dynamic)?
pub fn editable(index: u16) -> bool {
	(64..=159).contains(&index)
}

/// Is a slot color-cycled by the game?
pub fn animated(index: u16) -> bool {
	(9..=31).contains(&index) || (96..=127).contains(&index)
}

/// The section a slot belongs to.
pub fn section_of(index: u16) -> &'static Section {
	SECTIONS.iter().find(|s| index >= s.start && index <= s.end).expect("0-255 covered")
}

/// The water cycle block containing a slot, if any.
pub fn water_block(index: u16) -> Option<(u8, u8)> {
	u8::try_from(index).ok().and_then(|i| WATER_CYCLES.iter().copied().find(|&(s, e)| (s..=e).contains(&i)))
}

fn rows(section: &Section) -> u16 {
	(section.end - section.start + 1).div_ceil(COLS)
}

/// Swatch box size: sized to the panel width (the `COLS` columns fill it); the
/// grid scrolls vertically when it doesn't fit the height.
fn box_px(body: Rect) -> f32 {
	// Reserve the scrollbar gutter so swatches never sit under the bar.
	let by_w = (body.w - crate::ui::SCROLLBAR_W - 2.0 * PAD - (COLS - 1) as f32 * GAP) / COLS as f32;
	by_w.clamp(4.0, 28.0)
}

/// Total grid content height (labels + swatch rows).
fn grid_height(body: Rect) -> f32 {
	let b = box_px(body);
	let total_rows: u16 = SECTIONS.iter().map(rows).sum();
	2.0 * PAD + SECTIONS.len() as f32 * LABEL_H + total_rows as f32 * (b + GAP)
}

/// Panel chrome: the full Color Palette (toolbar with grid/saved tabs +
/// Save/Load) or the bare WRL Internal Palette (no toolbar, read-only) —
/// same grid, header, and editor-strip layout otherwise.
#[derive(Clone, Copy, PartialEq)]
enum Chrome {
	Full,
	Bare,
}

/// The grid's visible window (between the pinned header and editor strip)
/// — also the scissor rect for the scrolled quads.
pub fn grid_area(body: Rect) -> Rect {
	grid_area_at(body, Chrome::Full)
}

/// [`grid_area`] for the bare (WRL Internal Palette) panel.
pub fn grid_area_bare(body: Rect) -> Rect {
	grid_area_at(body, Chrome::Bare)
}

fn grid_area_at(body: Rect, chrome: Chrome) -> Rect {
	let c = inner_body(body, chrome);
	Rect::new(c.x, c.y + HEADER_H, c.w, (c.h - HEADER_H - EDITOR_H).max(0.0))
}

/// Top toolbar (pinned above everything): the grid/saved tabs + Save/Load.
pub const TAB_H: f32 = 20.0;
const SAVED_ROW: f32 = 18.0;

fn tab_bar(body: Rect) -> Rect {
	body.strip_top(TAB_H)
}

/// The content area below the toolbar (the bare panel has none) — where the
/// grid or the saved list lives.
fn inner_body(body: Rect, chrome: Chrome) -> Rect {
	let tab_h = match chrome {
		Chrome::Full => TAB_H,
		Chrome::Bare => 0.0,
	};
	Rect::new(body.x, body.y + tab_h, body.w, (body.h - tab_h).max(0.0))
}

/// Toolbar hit rects: `(grid tab, saved tab, Save, Load)`.
fn tab_rects(body: Rect) -> (Rect, Rect, Rect, Rect) {
	let (y, hh) = (body.y + 2.0, TAB_H - 4.0);
	(
		Rect::new(body.x + 2.0, y, 50.0, hh),
		Rect::new(body.x + 54.0, y, 52.0, hh),
		Rect::new(body.x + body.w - 92.0, y, 44.0, hh),
		Rect::new(body.x + body.w - 46.0, y, 44.0, hh),
	)
}

/// Row `i` of the saved-palettes list within the content area.
fn saved_row(inner: Rect, i: usize) -> Rect {
	Rect::new(inner.x + PAD, inner.y + PAD + i as f32 * SAVED_ROW, inner.w - 2.0 * PAD, SAVED_ROW - 1.0)
}

/// Draw the toolbar: grid/saved tabs (left) + Save/Load buttons (right).
fn draw_tab_bar(q: &mut UiQuads, body: Rect, show_saved: bool, w: f32, h: f32, hot: Hot) {
	q.material(tab_bar(body), w, h, theme::TITLE);
	let (grid, saved, save, load) = tab_rects(body);
	let ink = |on: bool| if on { theme::ACCENT } else { theme::INK_DIM };
	q.button_active(grid, w, h, !show_saved, hot);
	q.label_in("grid", grid, 6.0, crate::ui::FONT_SMALL, w, h, ink(!show_saved));
	q.button_active(saved, w, h, show_saved, hot);
	q.label_in("saved", saved, 6.0, crate::ui::FONT_SMALL, w, h, ink(show_saved));
	q.button(save, w, h, hot);
	q.label_in("save", save, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK);
	q.button(load, w, h, hot);
	q.label_in("load", load, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK);
}

/// Draw the saved-palettes list (one clickable row per palette file).
fn draw_saved_list(q: &mut UiQuads, saved: &[String], inner: Rect, w: f32, h: f32, hot: Hot) {
	q.material(inner, w, h, theme::PANEL);
	if saved.is_empty() {
		q.label(
			"no saved palettes found",
			inner.x + PAD,
			inner.y + PAD + 2.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		return;
	}
	for (i, name) in saved.iter().enumerate() {
		let r = saved_row(inner, i);
		if r.y + r.h > inner.y + inner.h {
			break; // overflow clips (no scroll yet)
		}
		q.field(r, w, h);
		if hot.hover(r) {
			q.rect(r, w, h, if hot.pressed(r) { theme::PRESS } else { theme::HOVER });
		}
		q.label_fit(name, r, 6.0, crate::ui::FONT_SMALL, w, h, theme::INK);
	}
}

/// Header buttons: `[cycle]` `[static]` — wired to the global palette
/// animation (`animate on|off`).
fn header_buttons(body: Rect, chrome: Chrome) -> (Rect, Rect) {
	let c = inner_body(body, chrome);
	let h = HEADER_H - 4.0;
	(Rect::new(c.x + 2.0, c.y + 2.0, 56.0, h), Rect::new(c.x + 60.0, c.y + 2.0, 56.0, h))
}

/// Scroll range so the last row can reach the grid-area bottom.
pub fn max_scroll(body: Rect) -> f32 {
	max_scroll_at(body, Chrome::Full)
}

/// [`max_scroll`] for the bare (WRL Internal Palette) panel.
pub fn max_scroll_bare(body: Rect) -> f32 {
	max_scroll_at(body, Chrome::Bare)
}

fn max_scroll_at(body: Rect, chrome: Chrome) -> f32 {
	crate::ui::scroll_max(grid_height(body), grid_area_at(body, chrome).h)
}

/// The editor strip rect (pinned at the body bottom, never scrolled).
fn editor_rect(body: Rect, chrome: Chrome) -> Rect {
	let c = inner_body(body, chrome);
	Rect::new(c.x, c.y + c.h - EDITOR_H, c.w, EDITOR_H)
}

/// One slider/bar row of three tracks; a row prefix occupies the left
/// margin. Rows: 0 = RGB sliders, 1 = HSL sliders, 2 = block HSL bars.
fn bar_rects(body: Rect, row: usize) -> [Rect; 3] {
	bar_rects_at(body, row, Chrome::Full)
}

fn bar_rects_at(body: Rect, row: usize, chrome: Chrome) -> [Rect; 3] {
	let e = editor_rect(body, chrome);
	let y = e.y + [22.0, 41.0, 60.0][row];
	let cell = (e.w - 2.0 * PAD - 36.0) / 3.0;
	[0, 1, 2].map(|i| Rect::new(e.x + PAD + 36.0 + i as f32 * cell + 11.0, y, (cell - 15.0).max(8.0), BAR_H))
}

/// Screen rect of a slot's swatch within the body, at a given scroll.
/// (Geometry probe — the click path goes through `slot_rect_at`; tests use it.)
#[allow(dead_code)]
pub fn slot_rect(body: Rect, scroll: f32, index: u16) -> Rect {
	slot_rect_at(body, scroll, index, Chrome::Full)
}

fn slot_rect_at(body: Rect, scroll: f32, index: u16, chrome: Chrome) -> Rect {
	let c = inner_body(body, chrome);
	let b = box_px(c);
	let scroll = scroll.clamp(0.0, max_scroll_at(body, chrome));
	let mut y = c.y + HEADER_H + PAD - scroll;
	for s in &SECTIONS {
		y += LABEL_H;
		if index >= s.start && index <= s.end {
			let i = index - s.start;
			return Rect::new(c.x + PAD + (i % COLS) as f32 * (b + GAP), y + (i / COLS) as f32 * (b + GAP), b, b);
		}
		y += rows(s) as f32 * (b + GAP);
	}
	unreachable!("sections cover 0-255");
}

/// The selected slot range `(lo, hi)` from the anchor + optional shift-end;
/// `lo == hi` is a single slot. `None` when nothing is selected.
pub fn selection(active: Option<u16>, sel_end: Option<u16>) -> Option<(u16, u16)> {
	let a = active?;
	Some(sel_end.map_or((a, a), |e| (a.min(e), a.max(e))))
}

/// Editable (dynamic, tileset-owned) slots inside an inclusive range.
pub fn editable_in(lo: u16, hi: u16) -> Vec<u16> {
	(lo..=hi).filter(|&i| editable(i)).collect()
}

/// What a click in the panel body does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
	Select(u16),
	/// Shift-click: extend the selection range to this slot.
	SelectTo(u16),
	/// Header button: turn palette cycling on/off (the global animation).
	Cycle(bool),
	/// Absolute slider for the selected color: channel 0..=2 = R/G/B,
	/// 3..=5 = H/S/L; `track` maps the cursor x to the value.
	Slider {
		channel: usize,
		track: Rect,
	},
	/// Relative drag re-tinting the whole water cycle: 0 H, 1 S, 2 L.
	BlockBar {
		channel: usize,
	},
	/// Switch the panel's tab: false = the grid, true = the saved-palettes list.
	ShowSaved(bool),
	/// Toolbar buttons: open the file dialog to save / load a palette.
	Save,
	Load,
	/// Click a row in the saved-palettes list — load that palette.
	LoadSaved(usize),
}

/// Hit-test a click: header buttons, then the editor sliders/bars (when the
/// selection can take palette edits — projects only; flat WRL docs are read-
/// only here), then the swatch grid. `sel_end` is the shift-click range end;
/// `shift` extends the selection on a grid click.
#[allow(clippy::too_many_arguments)]
pub fn click(
	body: Rect,
	active: Option<u16>,
	sel_end: Option<u16>,
	can_edit: bool,
	scroll: f32,
	x: f32,
	y: f32,
	shift: bool,
	show_saved: bool,
	saved_len: usize,
) -> Option<Action> {
	// Toolbar (topmost): grid/saved tabs + Save/Load.
	let (grid_tab, saved_tab, save, load) = tab_rects(body);
	if grid_tab.contains(x, y) {
		return Some(Action::ShowSaved(false));
	}
	if saved_tab.contains(x, y) {
		return Some(Action::ShowSaved(true));
	}
	if save.contains(x, y) {
		return Some(Action::Save);
	}
	if load.contains(x, y) {
		return Some(Action::Load);
	}
	if show_saved {
		let inner = inner_body(body, Chrome::Full);
		return (0..saved_len).find(|&i| saved_row(inner, i).contains(x, y)).map(Action::LoadSaved);
	}
	click_at(body, active, sel_end, can_edit, scroll, x, y, shift, Chrome::Full)
}

/// [`click`] for the bare (WRL Internal Palette) panel: no toolbar, no saved
/// list, read-only (selection + cycle buttons only).
pub fn click_bare(body: Rect, active: Option<u16>, sel_end: Option<u16>, scroll: f32, x: f32, y: f32, shift: bool) -> Option<Action> {
	click_at(body, active, sel_end, false, scroll, x, y, shift, Chrome::Bare)
}

#[allow(clippy::too_many_arguments)]
fn click_at(
	body: Rect,
	active: Option<u16>,
	sel_end: Option<u16>,
	can_edit: bool,
	scroll: f32,
	x: f32,
	y: f32,
	shift: bool,
	chrome: Chrome,
) -> Option<Action> {
	let (cycle, fixed) = header_buttons(body, chrome);
	if cycle.contains(x, y) {
		return Some(Action::Cycle(true));
	}
	if fixed.contains(x, y) {
		return Some(Action::Cycle(false));
	}
	if let Some((lo, hi)) = selection(active, sel_end).filter(|_| can_edit) {
		if lo != hi {
			// A multi-slot range: a single row of relative HSL bars shifts the
			// whole selection (the absolute RGB/HSL sliders only fit one slot).
			if !editable_in(lo, hi).is_empty() {
				for (i, r) in bar_rects_at(body, 0, chrome).iter().enumerate() {
					if r.contains(x, y) {
						return Some(Action::BlockBar { channel: i });
					}
				}
			}
		} else if editable(lo) {
			for row in 0..2 {
				for (i, r) in bar_rects_at(body, row, chrome).iter().enumerate() {
					if r.contains(x, y) {
						return Some(Action::Slider { channel: row * 3 + i, track: *r });
					}
				}
			}
			if water_block(lo).is_some() {
				for (i, r) in bar_rects_at(body, 2, chrome).iter().enumerate() {
					if r.contains(x, y) {
						return Some(Action::BlockBar { channel: i });
					}
				}
			}
		}
	}
	if !grid_area_at(body, chrome).contains(x, y) {
		return None;
	}
	let hit = (0..256u16).find(|&i| slot_rect_at(body, scroll, i, chrome).contains(x, y))?;
	Some(if shift && active.is_some() { Action::SelectTo(hit) } else { Action::Select(hit) })
}

/// sRGB byte → linear float (the UI pipeline works in linear).
fn srgb_to_linear(b: u8) -> f32 {
	let c = b as f32 / 255.0;
	if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Slot colors that repeat within the **dynamic** range (64–159) — wasted
/// editable slots, flagged per the design.
pub fn dynamic_duplicates(palette: &[u8]) -> Vec<u16> {
	let mut out = Vec::new();
	for i in 64..=159u16 {
		let a = &palette[i as usize * 3..i as usize * 3 + 3];
		let dup = (64..=159u16).any(|j| j != i && &palette[j as usize * 3..j as usize * 3 + 3] == a);
		if dup {
			out.push(i);
		}
	}
	out
}

/// One frame of palette content: the scrolled grid (drawn with `scissor`)
/// and the pinned chrome (header + editor strip, unclipped).
pub struct PaletteView {
	pub grid: UiQuads,
	pub chrome: UiQuads,
	pub scissor: Rect,
}

/// Build the panel. `display` colors the swatches (the live cycled palette
/// while cycling is on); `base` is the stored palette the editor strip and
/// duplicate detection read. `can_edit` = palette edits possible (project
/// open) — without it the strip is read-only (no sliders).
#[allow(clippy::too_many_arguments)]
pub fn view(
	display: &[u8],
	base: &[u8],
	active: Option<u16>,
	sel_end: Option<u16>,
	scroll: f32,
	cycling: bool,
	can_edit: bool,
	show_saved: bool,
	saved: &[String],
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> PaletteView {
	let mut chrome = UiQuads::with_steel_map(map);
	draw_tab_bar(&mut chrome, body, show_saved, w, h, hot);
	if show_saved {
		let inner = inner_body(body, Chrome::Full);
		draw_saved_list(&mut chrome, saved, inner, w, h, hot);
		return PaletteView { grid: UiQuads::default(), chrome, scissor: inner };
	}
	view_at(display, base, active, sel_end, scroll, cycling, can_edit, body, w, h, hot, chrome, Chrome::Full)
}

/// [`view`] for the bare (WRL Internal Palette) panel: the same grid, header,
/// and editor strip, but no toolbar and no editing — pure inspection.
#[allow(clippy::too_many_arguments)]
pub fn view_bare(
	display: &[u8],
	base: &[u8],
	active: Option<u16>,
	sel_end: Option<u16>,
	scroll: f32,
	cycling: bool,
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> PaletteView {
	let chrome = UiQuads::with_steel_map(map);
	view_at(display, base, active, sel_end, scroll, cycling, false, body, w, h, hot, chrome, Chrome::Bare)
}

#[allow(clippy::too_many_arguments)]
fn view_at(
	display: &[u8],
	base: &[u8],
	active: Option<u16>,
	sel_end: Option<u16>,
	scroll: f32,
	cycling: bool,
	can_edit: bool,
	body: Rect,
	w: f32,
	h: f32,
	hot: Hot,
	mut chrome: UiQuads,
	which: Chrome,
) -> PaletteView {
	let palette = display;
	let sel = selection(active, sel_end);
	let mut q = UiQuads::default();
	let b = box_px(body);
	let dups = dynamic_duplicates(base);
	let clip = grid_area_at(body, which);
	let scroll = scroll.clamp(0.0, max_scroll_at(body, which));
	let mut y = inner_body(body, which).y + HEADER_H + PAD - scroll;

	for s in &SECTIONS {
		let section_h = LABEL_H + rows(s) as f32 * (b + GAP);
		// Cull sections fully outside the visible window.
		if y + section_h < clip.y || y > clip.y + clip.h {
			y += section_h;
			continue;
		}
		let ink = if s.editable { theme::INK } else { theme::INK_DIM };
		q.label(s.label, body.x + PAD, y, crate::ui::FONT_SMALL, w, h, ink);
		if s.editable {
			// Editable sections carry an amber tick before the label line.
			q.rect(Rect::new(body.x + 1.0, y + 2.0, 2.0, LABEL_H - 4.0), w, h, theme::INK);
		}
		y += LABEL_H;

		for index in s.start..=s.end {
			let i = index - s.start;
			let r = Rect::new(body.x + PAD + (i % COLS) as f32 * (b + GAP), y + (i / COLS) as f32 * (b + GAP), b, b);
			let p = index as usize * 3;
			// Swatches are always opaque — they show the true colour; the
			// editable/fixed distinction is carried by the section label + tick.
			let color =
				[srgb_to_linear(palette[p]), srgb_to_linear(palette[p + 1]), srgb_to_linear(palette[p + 2]), 1.0];
			q.rect(r, w, h, color);
			if s.animated {
				// Animated slots: a small dot in the bottom-left corner.
				let dot = if s.editable { theme::INK } else { theme::INK_DIM };
				q.rect(Rect::new(r.x + 1.0, r.y + r.h - 3.0, 2.0, 2.0), w, h, dot);
			}
			if s.editable && dups.contains(&index) {
				// Duplicate color in the dynamic range: a warning corner.
				q.rect(Rect::new(r.x + r.w - 3.0, r.y + 1.0, 2.0, 2.0), w, h, theme::CLOSE_INK);
			}
			if sel.is_some_and(|(lo, hi)| (lo..=hi).contains(&index)) {
				q.border(Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), w, h, theme::INK);
			}
		}
		y += rows(s) as f32 * (b + GAP);
	}

	// Header: cycle / static buttons (the global palette animation).
	chrome.material(inner_body(body, which).strip_top(HEADER_H), w, h, theme::TITLE);
	let (cycle_btn, static_btn) = header_buttons(body, which);
	for (r, label, on) in [(cycle_btn, "cycle", cycling), (static_btn, "static", !cycling)] {
		chrome.button_active(r, w, h, on, hot);
		chrome.label_in(label, r, 6.0, crate::ui::FONT_SMALL, w, h, if on { theme::ACCENT } else { theme::INK_DIM });
	}
	chrome.material(editor_rect(body, which), w, h, theme::PANEL);
	draw_editor(&mut chrome, base, active, sel_end, can_edit, body, w, h, which);
	// Visible scrollbar over the grid window.
	chrome.scrollbar(clip, grid_height(body), scroll, w, h, hot);
	PaletteView { grid: q, chrome, scissor: clip }
}

/// The editor strip: selected-slot info, HSL bars (editable slots), and
/// the block HSL bars for water cycle slots.
#[allow(clippy::too_many_arguments)]
fn draw_editor(
	q: &mut UiQuads,
	palette: &[u8],
	active: Option<u16>,
	sel_end: Option<u16>,
	can_edit: bool,
	body: Rect,
	w: f32,
	h: f32,
	chrome: Chrome,
) {
	let e = editor_rect(body, chrome);
	q.rect(Rect::new(e.x, e.y - 1.0, e.w, 1.0), w, h, theme::PANEL_BORDER);

	// A multi-slot selection: one row of relative HSL bars re-tints every
	// editable slot in the range; absolute per-channel sliders only fit one slot.
	if let Some((lo, hi)) = selection(active, sel_end).filter(|&(lo, hi)| lo != hi) {
		let n = editable_in(lo, hi).len();
		// ASCII only (the MAX atlas has no em-dash), fitted to the strip width.
		let info = format!("{}-{hi} selected - {n} editable, drag to shift HSL", lo);
		let info = crate::text::fit_label(&info, crate::ui::FONT_SMALL, e.w - 2.0 * PAD);
		let live = can_edit && n > 0;
		let ink = if live { theme::INK } else { theme::INK_DIM };
		q.label(&info, e.x + PAD, e.y + 6.0, crate::ui::FONT_SMALL, w, h, ink);
		if live {
			let bars = bar_rects(body, 0);
			for (i, r) in bars.iter().enumerate() {
				q.label(["H", "S", "L"][i], r.x - 10.0, r.y + 1.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
				q.field(*r, w, h);
				// Relative bars: a center notch marks the rest position.
				q.rect(Rect::new(r.x + r.w / 2.0 - 1.0, r.y, 2.0, r.h), w, h, theme::INK);
			}
		}
		return;
	}

	let Some(slot) = active else {
		q.label("click a color to inspect/edit", e.x + PAD, e.y + 6.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		return;
	};
	let s = section_of(slot);
	let p = slot as usize * 3;
	let rgb = [palette[p], palette[p + 1], palette[p + 2]];
	let note = if s.editable { "" } else { "  (fixed)" };
	let info = format!("{slot}  #{:02x}{:02x}{:02x}  {}{note}", rgb[0], rgb[1], rgb[2], s.label);
	// Fit before the swatch preview at the strip's right edge.
	let info = crate::text::fit_label(&info, crate::ui::FONT_SMALL, e.w - PAD - 28.0);
	let live = s.editable && can_edit;
	q.label(&info, e.x + PAD, e.y + 6.0, crate::ui::FONT_SMALL, w, h, if live { theme::INK } else { theme::INK_DIM });
	// The selected color, full strength, beside the info line.
	let sw = Rect::new(e.x + e.w - 22.0, e.y + 4.0, 16.0, 16.0);
	q.rect(sw, w, h, [srgb_to_linear(rgb[0]), srgb_to_linear(rgb[1]), srgb_to_linear(rgb[2]), 1.0]);
	q.border(sw, w, h, theme::PANEL_BORDER);

	if !live {
		if s.editable && chrome == Chrome::Full {
			// A project slot without a project open: say why there are no
			// sliders (a flat WRL is read-only here). Fitted to the strip.
			// The bare panel is read-only by design — no note needed.
			let note = crate::text::fit_label(
				"read-only - open a project (.json) to edit",
				crate::ui::FONT_SMALL,
				e.w - 2.0 * PAD,
			);
			q.label(&note, e.x + PAD, e.y + 25.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
		return;
	}
	let (hue, sat, light) = rgb_to_hsl(rgb);
	// Row values: RGB as 0..1 fractions, then HSL.
	let values = [[rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0], [hue / 360.0, sat, light]];
	let letters = [["R", "G", "B"], ["H", "S", "L"]];
	for row in 0..2 {
		let bars = bar_rects(body, row);
		q.label(
			if row == 0 { "rgb" } else { "hsl" },
			e.x + PAD,
			bars[0].y + 1.0,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		for (i, r) in bars.iter().enumerate() {
			q.label(letters[row][i], r.x - 10.0, r.y + 1.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.field(*r, w, h);
			// Absolute sliders: fill = value, plus a 2px cursor notch.
			let v = values[row][i].clamp(0.0, 1.0);
			q.rect(Rect::new(r.x, r.y, r.w * v, r.h), w, h, theme::INK_DIM);
			q.rect(Rect::new(r.x + (r.w - 2.0) * v, r.y, 2.0, r.h), w, h, theme::INK);
		}
	}
	if water_block(slot).is_some() {
		let bars = bar_rects(body, 2);
		q.label("block", e.x + PAD, bars[0].y + 1.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		for (i, r) in bars.iter().enumerate() {
			q.label(letters[1][i], r.x - 10.0, r.y + 1.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
			q.field(*r, w, h);
			// Relative drag bars: a center notch marks the rest position.
			q.rect(Rect::new(r.x + r.w / 2.0 - 1.0, r.y, 2.0, r.h), w, h, theme::INK);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sections_cover_the_palette_exactly() {
		let mut next = 0u16;
		for s in &SECTIONS {
			assert_eq!(s.start, next, "no gap/overlap before '{}'", s.label);
			assert!(s.end >= s.start);
			next = s.end + 1;
		}
		assert_eq!(next, 256);
		// Contract classes (tileset-contract.md §1).
		assert!(editable(64) && editable(159) && !editable(63) && !editable(160));
		assert!(animated(9) && animated(31) && animated(96) && animated(127));
		assert!(!animated(8) && !animated(32) && !animated(95) && !animated(128));
	}

	#[test]
	fn slot_rects_round_trip_through_click() {
		let body = Rect::new(1000.0, 80.0, 280.0, 460.0);
		for scroll in [0.0, 120.0] {
			for &i in &[0u16, 8, 9, 63, 64, 100, 159, 160, 255] {
				let r = slot_rect(body, scroll, i);
				let g = grid_area(body);
				if r.y < g.y || r.y + r.h > g.y + g.h {
					continue; // scrolled out of the window — not clickable
				}
				match click(body, None, None, true, scroll, r.x + 1.0, r.y + 1.0, false, false, 0) {
					Some(Action::Select(got)) => assert_eq!(got, i, "slot {i} @ {scroll}"),
					_ => panic!("expected Select for slot {i} @ {scroll}"),
				}
			}
		}
		assert!(
			click(body, None, None, true, 0.0, body.x + 1.0, body.y + 1.0, false, false, 0).is_none(),
			"label strip"
		);
	}

	#[test]
	fn boxes_fill_the_width_and_the_grid_scrolls() {
		// Width-bound boxes: a wider panel → bigger swatches.
		let narrow = Rect::new(0.0, 0.0, 200.0, 300.0);
		let wide = Rect::new(0.0, 0.0, 420.0, 300.0);
		assert!(box_px(wide) > box_px(narrow));
		// A short body scrolls; the scroll range covers the full grid.
		assert!(max_scroll(narrow) > 0.0, "the 8-per-line grid can't fit a 200px window");
		let max = max_scroll(narrow);
		let last = slot_rect(narrow, max, 255);
		let g = grid_area(narrow);
		assert!(last.y + last.h <= g.y + g.h + 1.0, "fully scrolled, slot 255 visible");
		// The grid never paints into the toolbar / pinned header / editor (scissor).
		assert_eq!(g.h, narrow.h - TAB_H - HEADER_H - EDITOR_H);
		assert_eq!(g.y, narrow.y + TAB_H + HEADER_H);
		// A tall enough body needs no scroll at all (8-per-line is taller now).
		let tall = Rect::new(0.0, 0.0, 200.0, 1200.0);
		assert_eq!(max_scroll(tall), 0.0);
	}

	#[test]
	fn sliders_and_buttons_hit_correctly() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		let rgb = bar_rects(body, 0)[0];
		let hsl = bar_rects(body, 1)[1];
		let block = bar_rects(body, 2)[2];
		// Editable selection: RGB + HSL sliders live; water adds block bars.
		assert!(matches!(
			click(body, Some(100), None, true, 0.0, rgb.x + 2.0, rgb.y + 2.0, false, false, 0),
			Some(Action::Slider { channel: 0, .. }),
		));
		assert!(matches!(
			click(body, Some(100), None, true, 0.0, hsl.x + 2.0, hsl.y + 2.0, false, false, 0),
			Some(Action::Slider { channel: 4, .. }),
		));
		assert!(matches!(
			click(body, Some(100), None, true, 0.0, block.x + 2.0, block.y + 2.0, false, false, 0),
			Some(Action::BlockBar { channel: 2 }),
		));
		// Non-water editable slot: no block row.
		assert!(click(body, Some(70), None, true, 0.0, block.x + 2.0, block.y + 2.0, false, false, 0).is_none());
		// Fixed selection: no sliders at all (the strip area never selects).
		assert!(click(body, Some(32), None, true, 0.0, rgb.x + 2.0, rgb.y + 2.0, false, false, 0).is_none());
		// Header buttons route the cycle toggle.
		let (cycle, fixed) = header_buttons(body, Chrome::Full);
		assert!(matches!(
			click(body, None, None, true, 0.0, cycle.x + 2.0, cycle.y + 2.0, false, false, 0),
			Some(Action::Cycle(true)),
		));
		assert!(matches!(
			click(body, None, None, true, 0.0, fixed.x + 2.0, fixed.y + 2.0, false, false, 0),
			Some(Action::Cycle(false)),
		));
		// Contract helpers.
		assert_eq!(water_block(110), Some((110, 116)));
		assert_eq!(water_block(70), None);
		assert_eq!(section_of(100).label, "water cycle 96-102");
		assert_eq!(section_of(125).label, "water cycle 123-127");
	}

	#[test]
	fn shift_click_range_select_and_relative_bars() {
		// selection() resolves an ordered range; editable_in filters dynamic slots.
		assert_eq!(selection(Some(100), None), Some((100, 100)));
		assert_eq!(selection(Some(120), Some(100)), Some((100, 120)), "ordered low..high");
		assert_eq!(selection(None, Some(50)), None);
		assert_eq!(editable_in(60, 66), vec![64, 65, 66], "only 64.. are editable");

		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		// A multi-slot range exposes one row of relative HSL bars (BlockBar)…
		let bar = bar_rects(body, 0)[0];
		assert!(matches!(
			click(body, Some(64), Some(95), true, 0.0, bar.x + 2.0, bar.y + 2.0, false, false, 0),
			Some(Action::BlockBar { channel: 0 }),
		));
		// …while the same rect for a single editable slot is an absolute R slider.
		assert!(matches!(
			click(body, Some(64), None, true, 0.0, bar.x + 2.0, bar.y + 2.0, false, false, 0),
			Some(Action::Slider { channel: 0, .. }),
		));
		// Shift held on a visible grid swatch extends the selection.
		let r0 = slot_rect(body, 0.0, 0);
		if grid_area(body).contains(r0.x + 1.0, r0.y + 1.0) {
			assert!(matches!(
				click(body, Some(5), None, true, 0.0, r0.x + 1.0, r0.y + 1.0, true, false, 0),
				Some(Action::SelectTo(0)),
			));
			assert!(matches!(
				click(body, Some(5), None, true, 0.0, r0.x + 1.0, r0.y + 1.0, false, false, 0),
				Some(Action::Select(0)),
			));
		}
	}

	#[test]
	fn bare_panel_has_no_toolbar_and_never_edits() {
		let body = Rect::new(0.0, 0.0, 280.0, 460.0);
		// The grid starts one toolbar row higher than the full panel's.
		assert_eq!(grid_area_bare(body).y, body.y + HEADER_H);
		assert_eq!(grid_area(body).y, body.y + TAB_H + HEADER_H);
		assert_eq!(max_scroll(body) - max_scroll_bare(body), TAB_H.min(max_scroll(body)));
		// Swatches hit one toolbar row higher too — and round-trip to Select.
		let r = slot_rect_at(body, 0.0, 0, Chrome::Bare);
		assert_eq!(r.y, slot_rect(body, 0.0, 0).y - TAB_H);
		assert!(matches!(click_bare(body, None, None, 0.0, r.x + 1.0, r.y + 1.0, false), Some(Action::Select(0))));
		assert!(matches!(click_bare(body, Some(5), None, 0.0, r.x + 1.0, r.y + 1.0, true), Some(Action::SelectTo(0))));
		// The cycle/static header buttons live where the toolbar tabs sit in
		// the full panel — here they're the header.
		let (cycle, fixed) = header_buttons(body, Chrome::Bare);
		assert!(matches!(click_bare(body, None, None, 0.0, cycle.x + 2.0, cycle.y + 2.0, false), Some(Action::Cycle(true))));
		assert!(matches!(click_bare(body, None, None, 0.0, fixed.x + 2.0, fixed.y + 2.0, false), Some(Action::Cycle(false))));
		// No sliders/bars ever — even with an editable slot selected, the
		// rects that would be the R slider / block bars hit nothing.
		for row in 0..3 {
			let r = bar_rects_at(body, row, Chrome::Bare)[0];
			assert_eq!(click_bare(body, Some(100), None, 0.0, r.x + 2.0, r.y + 2.0, false), None, "row {row}");
		}
		// And the view builds without the toolbar (chrome quads still exist:
		// header + editor strip), with the bare scissor.
		let palette = vec![128u8; 768];
		let v = view_bare(&palette, &palette, Some(64), None, 0.0, false, body, 1280.0, 800.0, SteelMap::Stretch, Hot::NONE);
		assert!(!v.grid.verts.is_empty());
		assert!(!v.chrome.verts.is_empty());
		assert_eq!(v.scissor, grid_area_bare(body));
	}

	#[test]
	fn duplicate_detection_is_dynamic_range_only() {
		let mut palette = vec![0u8; 768];
		for i in 0..256usize {
			palette[i * 3] = i as u8; // unique reds
		}
		assert!(dynamic_duplicates(&palette).is_empty());
		// Same color at 70 and 130 (both dynamic) → both flagged.
		palette[70 * 3] = 7;
		palette[130 * 3] = 7;
		palette[70 * 3 + 1] = 9;
		palette[130 * 3 + 1] = 9;
		assert_eq!(dynamic_duplicates(&palette), vec![70, 130]);
		// A static slot repeating a dynamic color is not a warning.
		palette[130 * 3 + 1] = 10;
		palette[5 * 3] = 7;
		palette[5 * 3 + 1] = 9;
		assert!(dynamic_duplicates(&palette).is_empty());
	}

	#[test]
	fn full_click_chain_reaches_set_color() {
		// The whole press path, exactly as main.rs routes it: workspace
		// press → Press::Body("palette") → click → Slider → set_color.
		use crate::state::EditorState;
		use crate::workspace::Press;
		use map_core::Project;
		let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets");
		let project = Project::new(8, 8, &["GREEN".to_string()], &root, 42).unwrap();
		let mut editor = EditorState::new(project, (1280, 800), None, root.clone());
		editor.active_color = Some(100);

		let (w, h) = (1280.0, 800.0);
		let layout = editor.workspace.layout(w, h);
		let pi = editor.workspace.find("palette").unwrap();
		let r = layout.panels.iter().find(|(i, _)| *i == pi).unwrap().1;
		let body = editor.workspace.body_of(pi, r);
		let track = bar_rects(body, 0)[0]; // the R slider
		let (cx, cy) = (track.x + track.w * 0.75, track.y + 2.0);

		match editor.workspace.on_press(cx, cy, w, h) {
			Press::Body { id: "palette", body: b } => {
				assert_eq!(b, body);
				match click(b, editor.active_color.map(u16::from), None, true, 0.0, cx, cy, false, false, 0) {
					Some(Action::Slider { channel: 0, track: t }) => {
						let v = (((cx - t.x) / t.w).clamp(0.0, 1.0) * 255.0).round() as u8;
						let p = &mut editor.project;
						let at = 100 * 3;
						let rgb = [v, p.palette[at + 1], p.palette[at + 2]];
						assert!(p.set_color(100, rgb).unwrap());
						assert_eq!(p.palette[at], v);
					}
					_ => panic!("expected the R slider at ({cx},{cy}) in {t:?}", t = track),
				}
			}
			other => panic!("expected palette body at ({cx},{cy}), got {other:?}"),
		}
	}

	#[test]
	fn view_splits_scrolled_grid_from_pinned_chrome() {
		let palette = vec![128u8; 768];
		// Short body → deep scroll → whole top sections get culled.
		let body = Rect::new(0.0, 0.0, 280.0, 300.0);
		let v = view(
			&palette,
			&palette,
			Some(64),
			None,
			0.0,
			false,
			true,
			false,
			&[],
			body,
			1280.0,
			800.0,
			SteelMap::Stretch,
			Hot::NONE,
		);
		assert!(!v.grid.verts.is_empty());
		assert!(!v.chrome.verts.is_empty(), "header + editor strip drawn");
		assert_eq!(v.scissor, grid_area(body));
		// Scrolled to the end, the top sections are culled → fewer quads.
		let scrolled = view(
			&palette,
			&palette,
			Some(64),
			None,
			max_scroll(body),
			false,
			true,
			false,
			&[],
			body,
			1280.0,
			800.0,
			SteelMap::Stretch,
			Hot::NONE,
		);
		if max_scroll(body) > 0.0 {
			assert_ne!(scrolled.grid.verts.len(), v.grid.verts.len());
		}
	}
}
