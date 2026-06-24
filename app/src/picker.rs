//! Tile Explorer content: the pickable-tile list (filters),
//! grid layout/hit-testing, and the per-frame view (tile quads for the GPU
//! pass + chrome overlay). Pure logic - the GPU half lives in
//! `project_render::draw_picker`, input routing in `main.rs`.

use std::collections::HashSet;

use map_core::Project;

use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};

/// Display sizes the size dropdown offers (the larger ones suit a wide panel
/// or close inspection of a single tile).
pub const SIZES: [f32; 7] = [16.0, 24.0, 32.0, 48.0, 64.0, 128.0, 256.0];
/// Height of one header control row.
const ROW_H: f32 = 20.0;
const PAD: f32 = 4.0;
const GAP: f32 = 2.0;
/// Wheel scroll per notch (px).
pub const WHEEL_STEP: f32 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
	All,
	Used,
	Unused,
	Water,
	Shore,
	Land,
	Blocked,
}

impl Filter {
	pub const ALL: [Filter; 7] =
		[Filter::All, Filter::Used, Filter::Unused, Filter::Water, Filter::Shore, Filter::Land, Filter::Blocked];

	pub fn name(self) -> &'static str {
		match self {
			Filter::All => "all",
			Filter::Used => "used",
			Filter::Unused => "unused",
			Filter::Water => "water",
			Filter::Shore => "shore",
			Filter::Land => "land",
			Filter::Blocked => "blocked",
		}
	}

	pub fn next(self) -> Filter {
		let i = Self::ALL.iter().position(|&f| f == self).unwrap_or(0);
		Self::ALL[(i + 1) % Self::ALL.len()]
	}

	pub fn parse(s: &str) -> Option<Filter> {
		Self::ALL.iter().copied().find(|f| f.name() == s)
	}

	/// The pass value this filter selects (0 land / 1 water / 2 shore /
	/// 3 blocked) - `None` for the non-pass filters.
	fn pass(self) -> Option<u8> {
		match self {
			Filter::Land => Some(0),
			Filter::Water => Some(1),
			Filter::Shore => Some(2),
			Filter::Blocked => Some(3),
			_ => None,
		}
	}
}

pub struct PickerState {
	pub tile_px: f32,
	pub scroll: f32,
	pub filter: Filter,
	/// The filter dropdown's open state.
	pub filter_open: bool,
	/// The size dropdown's open state.
	pub size_open: bool,
}

impl Default for PickerState {
	fn default() -> Self {
		Self { tile_px: 32.0, scroll: 0.0, filter: Filter::All, filter_open: false, size_open: false }
	}
}

impl PickerState {
	pub fn cycle_size(&mut self) {
		let i = SIZES.iter().position(|&s| s == self.tile_px).unwrap_or(2);
		self.tile_px = SIZES[(i + 1) % SIZES.len()];
	}
}

/// One pickable tile.
pub struct Item<'a> {
	/// Pack/tile coordinates - tests pin the contract; the eyedropper and
	/// group filters (the custom group filter) are the future readers.
	#[allow(dead_code)]
	pub pack: usize,
	#[allow(dead_code)]
	pub tile: u16,
	pub id: &'a str,
	/// Global atlas index (`sum of preceding packs' tile counts + tile`) -
	/// the same contract `project_render::build_cell_data` uses.
	pub index: u32,
}

/// The project's tiles under `filter`, in pack order.
pub fn items(project: &Project, filter: Filter) -> Vec<Item<'_>> {
	let used: Option<HashSet<(u8, u16)>> = match filter {
		Filter::Used | Filter::Unused => {
			Some(project.cells.iter().flat_map(|stack| stack.iter().flatten()).map(|t| (t.pack, t.tile)).collect())
		}
		_ => None,
	};

	let mut out = Vec::new();
	let mut base = 0u32;
	for (pack_index, pack) in project.packs.iter().enumerate() {
		for tile in 0..pack.tile_count() {
			let keep = match filter {
				Filter::All => true,
				Filter::Used => used.as_ref().is_some_and(|u| u.contains(&(pack_index as u8, tile))),
				Filter::Unused => used.as_ref().is_some_and(|u| !u.contains(&(pack_index as u8, tile))),
				f => pack.pass.as_ref().is_some_and(|pass| Some(pass[tile as usize]) == f.pass()),
			};
			if keep {
				out.push(Item { pack: pack_index, tile, id: &pack.ids[tile as usize], index: base + tile as u32 });
			}
		}
		base += pack.tile_count() as u32;
	}
	out
}

/// Grid geometry for the picker body (header row on top, `tile_px` cells below).
pub fn grid(body: Rect, tile_px: f32) -> crate::cellgrid::Grid {
	crate::cellgrid::Grid { body, cell: tile_px, gap: GAP, pad: PAD, header: header_h(body), row_extra: 0.0 }
}

/// Screen rect of item `i` at a given scroll.
pub fn item_rect(g: &crate::cellgrid::Grid, _tile_px: f32, scroll: f32, i: usize) -> Rect {
	g.item_rect(i, scroll)
}

/// Scroll range so the last row can reach the body bottom.
pub fn max_scroll(count: usize, body: Rect, tile_px: f32) -> f32 {
	grid(body, tile_px).max_scroll(count)
}

/// Scroll offset that brings item `index` into the grid's visible window,
/// moving as little as possible from `scroll` (a no-op when it's already shown).
/// Used to reveal the just-picked tile (the map eyedropper).
pub fn scroll_to_reveal(body: Rect, tile_px: f32, count: usize, index: usize, scroll: f32) -> f32 {
	let g = grid(body, tile_px);
	let max = g.max_scroll(count);
	let top = g.item_rect(index, scroll).y; // current on-screen top
	let bot = top + tile_px;
	let (win_top, win_bot) = (body.y + header_h(body), body.y + body.h);
	let s = if top < win_top {
		scroll - (win_top - top) // scroll up to bring the top into view
	} else if bot > win_bot {
		scroll + (bot - win_bot) // scroll down to bring the bottom into view
	} else {
		scroll // already visible
	};
	s.clamp(0.0, max)
}

/// The grid's clip area (body minus the header rows).
pub fn scissor(body: Rect) -> Rect {
	crate::cellgrid::scissor(body, header_h(body))
}

/// The filter dropdown's fixed width: its longest option plus the caret gutter
/// and padding (see [`crate::select::draw_box`]), so the closed value and the
/// popup options never ellipsize.
fn filter_w() -> f32 {
	let longest =
		Filter::ALL.iter().map(|f| crate::text::label_width(f.name(), crate::ui::FONT_SMALL)).fold(0.0_f32, f32::max);
	longest + 28.0
}

/// The size dropdown's fixed width: enough that the popup options (`"256 px"`)
/// render in full (the closed number then fits trivially).
fn size_w() -> f32 {
	let longest = SIZES
		.iter()
		.map(|s| crate::text::label_width(&format!("{} px", *s as u32), crate::ui::FONT_SMALL))
		.fold(0.0_f32, f32::max);
	longest + 12.0
}

/// The fixed width shared by the `new`/`clone`/`edit`/`delete` buttons (sized to
/// the longest label).
fn action_w() -> f32 {
	let longest = ["new", "clone", "edit", "delete"]
		.iter()
		.map(|s| crate::text::label_width(s, crate::ui::FONT_SMALL))
		.fold(0.0_f32, f32::max);
	longest + 12.0
}

/// Right-edge space reserved on the first header row for the tile count, so the
/// controls never collide with it (sized for up to four digits).
fn count_reserve() -> f32 {
	crate::text::label_width("0000", crate::ui::FONT_SMALL) + 6.0 + GAP
}

/// The header controls in flow order - the filter + size selects, then the four
/// action buttons - each a fixed width and wrapping to a new row when it won't
/// fit. Row 0 keeps clear of the tile count on the right. Returns one rect per
/// control (filter, size, new, clone, edit, delete) and the total row count.
fn header_flow(body: Rect) -> (Vec<Rect>, usize) {
	let widths = [filter_w(), size_w(), action_w(), action_w(), action_w(), action_w()];
	let left = body.x + PAD;
	let mut rects = Vec::with_capacity(widths.len());
	let mut x = left;
	let mut row = 0usize;
	for &cw in &widths {
		let right = body.x + body.w - if row == 0 { count_reserve() } else { PAD };
		if x > left && x + cw > right {
			row += 1;
			x = left;
		}
		rects.push(Rect::new(x, body.y + 2.0 + row as f32 * ROW_H, cw, ROW_H - 2.0));
		x += cw + GAP;
	}
	(rects, row + 1)
}

/// The header band height: as many control rows as the flow needs.
fn header_h(body: Rect) -> f32 {
	header_flow(body).1 as f32 * ROW_H + 4.0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
	/// Make this tile id the active brush.
	Pick(String),
	/// Pick a filter from the open dropdown (closes it).
	SetFilter(Filter),
	/// Toggle the filter dropdown open/closed.
	ToggleFilter,
	/// A click off an open dropdown closes it (eats the click).
	CloseFilter,
	/// Toggle the size dropdown open/closed.
	ToggleSize,
	/// Pick size option `i` (index into [`SIZES`]); closes the dropdown.
	SetSize(usize),
	/// A click off the open size dropdown closes it.
	CloseSize,
	/// Open the Tile Painter on a blank new tile.
	NewTile,
	/// Open the Tile Painter cloning the selected tile.
	CloneTile,
	/// Open the Tile Painter editing the selected tile.
	EditTile,
	/// Delete the selected tile from its pack.
	DeleteTile,
}

/// What a click at `(x, y)` in the panel body does.
pub fn click(project: &Project, state: &PickerState, body: Rect, x: f32, y: f32) -> Option<Action> {
	let ctrls = header_flow(body).0;
	let (filter_btn, size_btn) = (ctrls[0], ctrls[1]);
	// The filter dropdown takes priority while open - its list floats over the
	// grid below the header.
	match crate::select::hit(filter_btn, state.filter_open, Filter::ALL.len(), false, x, y) {
		Some(crate::select::Hit::Box) => return Some(Action::ToggleFilter),
		Some(crate::select::Hit::Option(i)) => return Some(Action::SetFilter(Filter::ALL[i])),
		None if state.filter_open => return Some(Action::CloseFilter),
		None => {}
	}
	match crate::select::hit(size_btn, state.size_open, SIZES.len(), false, x, y) {
		Some(crate::select::Hit::Box) => return Some(Action::ToggleSize),
		Some(crate::select::Hit::Option(i)) => return Some(Action::SetSize(i)),
		None if state.size_open => return Some(Action::CloseSize),
		None => {}
	}
	for (r, action) in ctrls[2..].iter().zip([Action::NewTile, Action::CloneTile, Action::EditTile, Action::DeleteTile])
	{
		if r.contains(x, y) {
			return Some(action);
		}
	}
	if y < body.y + header_h(body) {
		return None;
	}
	let list = items(project, state.filter);
	let g = grid(body, state.tile_px);
	let scroll = state.scroll.clamp(0.0, max_scroll(list.len(), body, state.tile_px));
	let i = g.index_at(x, y, scroll)?;
	let r = item_rect(&g, state.tile_px, scroll, i);
	// Inside the tile proper (not the gap), and a real item.
	(r.contains(x, y) && i < list.len()).then(|| Action::Pick(list[i].id.to_string()))
}

/// A tile quad for the GPU grid pass (`transform` = map-core bits; the
/// grid passes 0 = base art, the toolbox preview the active transform).
pub struct TileQuad {
	pub index: u32,
	pub transform: u32,
	pub rect: Rect,
}

/// The global atlas index of a tile ref - the same cumulative-pack-base
/// contract `project_render::build_cell_data` uses.
pub fn global_index(project: &Project, t: map_core::TileRef) -> u32 {
	let base: u32 = project.packs[..t.pack as usize].iter().map(|p| p.tile_count() as u32).sum();
	base + t.tile as u32
}

/// One frame of picker content for a panel body.
pub struct View {
	pub tiles: Vec<TileQuad>,
	pub overlay: UiQuads,
	pub scissor: Rect,
}

/// Build the visible tile quads + chrome overlay. `active` is the current
/// brush spec (transform suffix tolerated) for the selection highlight.
#[allow(clippy::too_many_arguments)]
pub fn view(
	project: &Project,
	state: &PickerState,
	active: Option<&str>,
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> View {
	let list = items(project, state.filter);
	let g = grid(body, state.tile_px);
	let scroll = state.scroll.clamp(0.0, max_scroll(list.len(), body, state.tile_px));
	let clip = scissor(body);
	let active_id = active.map(|s| s.split(':').next().unwrap_or(s));

	let mut tiles = Vec::new();
	let mut overlay = UiQuads::with_steel_map(map);
	for (i, item) in list.iter().enumerate() {
		let r = item_rect(&g, state.tile_px, scroll, i);
		if r.y + r.h < clip.y || r.y > clip.y + clip.h {
			continue;
		}
		tiles.push(TileQuad { index: item.index, transform: 0, rect: r });
		if active_id == Some(item.id) {
			// Selection ring (clamped to the clip area by geometry: a ring
			// one px outside the tile, drawn over the grid).
			let ring = Rect::new(r.x - 1.0, r.y.max(clip.y) - 1.0, r.w + 2.0, r.h + 2.0);
			overlay.border(ring, w, h, theme::INK);
		} else if hot.hover(r) && r.y >= clip.y {
			// Hover ring on the cell under the cursor (dimmer than selection).
			overlay.border(Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), w, h, theme::INK_DIM);
		}
	}

	// Header: filter dropdown + size dropdown + the action buttons (flowed,
	// fixed-width), and the count, over a steel sub-toolbar.
	overlay.material(body.strip_top(header_h(body)), w, h, theme::TITLE);
	let ctrls = header_flow(body).0;
	let (filter_btn, size_btn) = (ctrls[0], ctrls[1]);
	crate::select::draw_box(&mut overlay, filter_btn, state.filter.name(), state.filter_open, w, h, hot);
	// Closed box shows just the number (the popup spells out "N px").
	crate::select::draw_box(&mut overlay, size_btn, &format!("{}", state.tile_px as u32), state.size_open, w, h, hot);
	// Action buttons: new (always), clone/edit/delete (need a selected tile -
	// greyed otherwise; the command still reports why if clicked).
	let has = active.is_some();
	for (r, label) in ctrls[2..].iter().zip(["new", "clone", "edit", "delete"]) {
		let live = has || label == "new";
		if live {
			overlay.button(*r, w, h, hot);
		} else {
			overlay.button_disabled(*r, w, h);
		}
		overlay.label_in(label, *r, 6.0, crate::ui::FONT_SMALL, w, h, if live { theme::INK } else { theme::INK_DIM });
	}
	let count = format!("{}", list.len());
	let cx = body.x + body.w - 6.0 - crate::text::label_width(&count, crate::ui::FONT_SMALL);
	overlay.label(&count, cx, body.y + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);

	// Visible scrollbar over the tile grid. Content height within the clip
	// window mirrors `max_scroll` (which measures against the full body).
	overlay.scrollbar(clip, g.content_height(list.len()), scroll, w, h, hot);

	// The dropdown lists float over the grid - drawn last so they sit on top
	// (only one is open at a time).
	if state.filter_open {
		let labels: Vec<&str> = Filter::ALL.iter().map(|f| f.name()).collect();
		let selected = Filter::ALL.iter().position(|&f| f == state.filter);
		crate::select::draw_popup(&mut overlay, filter_btn, &labels, selected, false, w, h, hot);
	}
	if state.size_open {
		let labels: Vec<String> = SIZES.iter().map(|s| format!("{} px", *s as u32)).collect();
		let selected = SIZES.iter().position(|&s| s == state.tile_px);
		crate::select::draw_popup(&mut overlay, size_btn, &labels, selected, false, w, h, hot);
	}

	View { tiles, overlay, scissor: clip }
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::Path;

	fn project() -> Project {
		let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources/assets/tilepacks");
		Project::new(8, 6, &["GREEN".to_string()], &root, 42).unwrap()
	}

	#[test]
	fn filters_partition_the_packs() {
		let p = project();
		let total: usize = p.packs.iter().map(|k| k.tile_count() as usize).sum();
		assert_eq!(items(&p, Filter::All).len(), total);

		// Pass filters cover every tile exactly once (both packs ship pass data).
		let by_pass: usize =
			[Filter::Water, Filter::Shore, Filter::Land, Filter::Blocked].iter().map(|&f| items(&p, f).len()).sum();
		assert_eq!(by_pass, total);

		// A fresh map uses only water variants.
		let used = items(&p, Filter::Used);
		assert!(!used.is_empty() && used.len() <= 12);
		assert!(used.iter().all(|i| i.id.starts_with("WTR")));
		assert_eq!(items(&p, Filter::Unused).len(), total - used.len());

		// Atlas indices follow the pack_base contract (WATER first).
		let all = items(&p, Filter::All);
		assert_eq!((all[0].pack, all[0].tile, all[0].index), (0, 0, 0));
		let first_green = all.iter().find(|i| i.pack == 1).unwrap();
		assert_eq!(first_green.tile, 0);
		assert_eq!(first_green.index, p.packs[0].tile_count() as u32);
	}

	#[test]
	fn grid_hit_round_trips() {
		let p = project();
		let state = PickerState { tile_px: 32.0, scroll: 100.0, ..Default::default() };
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let list = items(&p, state.filter);
		let g = grid(body, state.tile_px);
		let scroll = state.scroll.clamp(0.0, max_scroll(list.len(), body, state.tile_px));
		for &i in &[0usize, 7, 8, 100, list.len() - 1] {
			let r = item_rect(&g, state.tile_px, scroll, i);
			if r.y < body.y + header_h(body) || r.y + r.h > body.y + body.h {
				continue; // scrolled out of view - not clickable
			}
			match click(&p, &state, body, r.x + 5.0, r.y + 5.0) {
				Some(Action::Pick(id)) => assert_eq!(id, list[i].id, "item {i}"),
				_ => panic!("expected Pick for item {i}"),
			}
		}
		// The gap between tiles picks nothing.
		let r = item_rect(&g, state.tile_px, scroll, g.cols() + 1);
		assert!(click(&p, &state, body, r.x - 1.0, r.y + 5.0).is_none());
	}

	#[test]
	fn header_controls_and_filter_dropdown() {
		let p = project();
		let body = Rect::new(1000.0, 50.0, 278.0, 500.0);
		let ctrls = header_flow(body).0;
		let (filter_btn, size_btn) = (ctrls[0], ctrls[1]);
		// Closed: clicking each box toggles its dropdown.
		let closed = PickerState::default();
		assert_eq!(click(&p, &closed, body, filter_btn.x + 2.0, filter_btn.y + 2.0), Some(Action::ToggleFilter));
		assert_eq!(click(&p, &closed, body, size_btn.x + 2.0, size_btn.y + 2.0), Some(Action::ToggleSize));
		// Size dropdown open: each row picks its size.
		let sized = PickerState { size_open: true, ..PickerState::default() };
		for i in 0..SIZES.len() {
			let o = crate::select::option_rect(size_btn, i, SIZES.len(), false);
			assert_eq!(click(&p, &sized, body, o.x + 2.0, o.y + 2.0), Some(Action::SetSize(i)));
		}

		// Open: each option row picks its filter; a click off the list closes it.
		let open = PickerState { filter_open: true, ..PickerState::default() };
		for (i, &f) in Filter::ALL.iter().enumerate() {
			let o = crate::select::option_rect(filter_btn, i, Filter::ALL.len(), false);
			assert_eq!(click(&p, &open, body, o.x + 2.0, o.y + 2.0), Some(Action::SetFilter(f)));
		}
		// A click on a tile while open just closes the dropdown (eats the click).
		assert_eq!(click(&p, &open, body, body.x + 20.0, body.y + 300.0), Some(Action::CloseFilter));

		assert_eq!(Filter::All.next(), Filter::Used);
		assert_eq!(Filter::Blocked.next(), Filter::All);
		assert_eq!(Filter::parse("shore"), Some(Filter::Shore));
		assert_eq!(Filter::parse("nope"), None);

		let mut s = PickerState::default();
		s.cycle_size();
		assert_eq!(s.tile_px, 48.0);
	}

	#[test]
	fn header_controls_are_fixed_width_and_wrap() {
		let narrow = Rect::new(0.0, 0.0, 200.0, 400.0);
		let wide = Rect::new(0.0, 0.0, 600.0, 400.0);
		let (n, n_rows) = header_flow(narrow);
		let (wd, w_rows) = header_flow(wide);
		// Controls keep a fixed width regardless of the panel size.
		for i in 0..6 {
			assert_eq!(n[i].w, wd[i].w, "control {i} keeps a fixed width");
		}
		// Selects fit their longest option (filter "blocked", size "256 px").
		assert!(n[0].w >= crate::text::label_width("blocked", crate::ui::FONT_SMALL));
		assert!(n[1].w >= crate::text::label_width("256 px", crate::ui::FONT_SMALL) + 10.0);
		// Wide → one row; narrow → wraps to more rows.
		assert_eq!(w_rows, 1, "all controls fit one row when wide");
		assert!(n_rows > 1, "a narrow panel wraps controls to new rows");
		// No row-0 control overlaps the tile-count area on the right.
		let count_left = wide.x + wide.w - count_reserve();
		for r in wd.iter().filter(|r| (r.y - (wide.y + 2.0)).abs() < 0.5) {
			assert!(r.x + r.w <= count_left + 0.5, "row-0 control clears the count");
		}
	}

	#[test]
	fn view_culls_to_the_body_and_highlights_the_active_tile() {
		let p = project();
		let state = PickerState::default();
		let body = Rect::new(1000.0, 50.0, 278.0, 300.0);
		let total = items(&p, Filter::All).len();
		let v = view(&p, &state, Some("WTR003:!N"), body, 1280.0, 800.0, SteelMap::Stretch, Hot::NONE);
		assert!(v.tiles.len() < total, "off-screen rows are culled");
		assert!(!v.tiles.is_empty());
		// All emitted quads at least touch the clip area.
		for t in &v.tiles {
			assert!(t.rect.y + t.rect.h >= v.scissor.y && t.rect.y <= v.scissor.y + v.scissor.h);
		}
		// Selection ring present: WTR003 is on the first screen (border = 4
		// rects = 24 verts in the overlay before the header strip).
		assert!(v.overlay.verts.len() > 24);
	}

	#[test]
	fn scroll_to_reveal_brings_an_offscreen_item_into_view() {
		let body = Rect::new(0.0, 0.0, 278.0, 200.0); // small → must scroll
		let (count, tile_px) = (500usize, 32.0);
		let g = grid(body, tile_px);
		let win = scissor(body);
		assert!(max_scroll(count, body, tile_px) > 0.0);
		// A far-down item: revealing it lands it fully inside the grid window.
		let i = 400;
		let s = scroll_to_reveal(body, tile_px, count, i, 0.0);
		let r = item_rect(&g, tile_px, s, i);
		assert!(r.y >= win.y - 0.5 && r.y + r.h <= win.y + win.h + 0.5, "item {i} revealed");
		// An already-visible item doesn't move the scroll.
		assert_eq!(scroll_to_reveal(body, tile_px, count, 0, 0.0), 0.0, "top item needs no scroll");
	}

	#[test]
	fn scroll_clamps_to_content() {
		let p = project();
		let body = Rect::new(0.0, 0.0, 278.0, 500.0);
		let count = items(&p, Filter::All).len();
		let max = max_scroll(count, body, 32.0);
		assert!(max > 0.0, "421 tiles don't fit a 500px body");
		assert_eq!(max_scroll(8, body, 32.0), 0.0, "one row never scrolls");
	}
}
