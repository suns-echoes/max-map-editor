//! Unit sprite library + Units panel (palette-tuning aid): load unit and
//! building sprites from the user's own game data (`MaxPath`/MAX.RES),
//! list them in a picker-style grid, and stamp non-document *preview*
//! placements on the map so palette edits can be judged against real units.
//!
//! Pure logic — the GPU half (atlas + quad pass) lives in `units_render.rs`,
//! input routing in `main.rs`. Format knowledge (multi-image strips, `D_*`
//! base records, `S_*` shadow strips, team-color slots) follows re-MAX.

use std::path::Path;

use max_assets::image::{IndexedFrame, decode_multi_image_indexed, decode_multi_image_shadow_indexed};
use max_assets::res::{read_res_entry, read_res_index};
use max_assets::units::{BaseUnitData, parse_base_unit_data};

use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};
use crate::units_render::AtlasSlots;

/// The five player colors of the original game, in remap-table order.
pub const TEAMS: usize = 5;
pub const TEAM_NAMES: [&str; TEAMS] = ["red", "green", "blue", "gray", "yellow"];
/// Swatch colors for the team picker row (UI only — sprites recolor through
/// the palette remap in `units.wgsl`, not these).
pub const TEAM_SWATCH: [[f32; 4]; TEAMS] = [
	[0.78, 0.16, 0.16, 1.0],
	[0.20, 0.62, 0.22, 1.0],
	[0.22, 0.38, 0.78, 1.0],
	[0.55, 0.55, 0.55, 1.0],
	[0.80, 0.72, 0.25, 1.0],
];

/// Sprites larger than this don't fit an atlas slot and are skipped (the
/// biggest 2×2 buildings are 128 px; anything beyond is intro/FX art).
pub const MAX_SPRITE: u32 = 128;

/// Per-unit sprite-strip layout: `(tag, body_base, body_count, turret_base,
/// turret_count)`. Extracted from re-MAX's `art.ini`, which in turn dumps the
/// original game's base-unit-data — the `D_*` records in MAX.RES are shared
/// per-*class* templates, so per-unit truth has to come from a table like
/// this (fixed turrets keep their turret strip at frame 1, not 8; SCANNER /
/// AWAC turret strips spin through 16/30 frames; …). Explosion/projectile FX
/// are deliberately omitted — they aren't placeable map dressing.
const STRIPS: &[(&str, u8, u8, u8, u8)] = &[
	("COMMTWR", 0, 2, 0, 0),
	("POWERSTN", 0, 2, 0, 0),
	("POWGEN", 0, 2, 0, 0),
	("BARRACKS", 0, 2, 0, 0),
	("SHIELDGN", 0, 2, 0, 0),
	("RADAR", 0, 16, 0, 0),
	("ADUMP", 0, 2, 0, 0),
	("FDUMP", 0, 2, 0, 0),
	("GOLDSM", 0, 2, 0, 0),
	("DEPOT", 0, 2, 0, 0),
	("HANGAR", 0, 2, 0, 0),
	("DOCK", 0, 2, 0, 0),
	("CNCT_4W", 0, 2, 0, 0),
	("LRGRUBLE", 0, 2, 0, 0),
	("SMLRUBLE", 0, 5, 0, 0),
	("LRGTAPE", 0, 2, 0, 0),
	("SMLTAPE", 0, 2, 0, 0),
	("LRGSLAB", 0, 5, 0, 0),
	("SMLSLAB", 0, 1, 0, 0),
	("LRGCONES", 0, 1, 0, 0),
	("SMLCONES", 0, 1, 0, 0),
	("ROAD", 0, 1, 0, 0),
	("LANDPAD", 0, 1, 0, 0),
	("SHIPYARD", 0, 2, 0, 0),
	("LIGHTPLT", 0, 2, 0, 0),
	("LANDPLT", 0, 2, 0, 0),
	("SUPRTPLT", 0, 2, 0, 0),
	("AIRPLT", 0, 2, 0, 0),
	("HABITAT", 0, 2, 0, 0),
	("RESEARCH", 0, 2, 0, 0),
	("GREENHSE", 0, 2, 0, 0),
	("RECCENTR", 0, 2, 0, 0),
	("TRAINHAL", 0, 2, 0, 0),
	("WTRPLTFM", 0, 1, 0, 0),
	("GUNTURRT", 0, 1, 1, 8),
	("ANTIAIR", 0, 1, 1, 8),
	("ARTYTRRT", 0, 1, 1, 8),
	("ANTIMSSL", 0, 1, 1, 8),
	("BLOCK", 0, 2, 0, 0),
	("BRIDGE", 0, 4, 0, 0),
	("MININGST", 0, 16, 0, 0),
	("LANDMINE", 0, 1, 0, 0),
	("SEAMINE", 0, 1, 0, 0),
	("CONSTRCT", 0, 16, 0, 0),
	("SCOUT", 0, 16, 0, 0),
	("TANK", 0, 8, 8, 8),
	("ARTILLRY", 0, 8, 0, 0),
	("ROCKTLCH", 0, 8, 0, 0),
	("MISSLLCH", 0, 8, 0, 0),
	("SP_FLAK", 0, 8, 8, 8),
	("MINELAYR", 0, 8, 0, 0),
	("SURVEYOR", 0, 16, 0, 0),
	("SCANNER", 0, 8, 8, 16),
	("SPLYTRCK", 0, 8, 0, 0),
	("GOLDTRCK", 0, 8, 0, 0),
	("ENGINEER", 0, 16, 0, 0),
	("BULLDOZR", 0, 8, 0, 0),
	("REPAIR", 0, 8, 0, 0),
	("FUELTRCK", 0, 8, 0, 0),
	("CLNTRANS", 0, 8, 0, 0),
	("COMMANDO", 0, 208, 0, 0),
	("INFANTRY", 0, 200, 0, 0),
	("FASTBOAT", 0, 8, 8, 8),
	("CORVETTE", 0, 8, 0, 0),
	("BATTLSHP", 0, 8, 8, 8),
	("SUBMARNE", 0, 16, 0, 0),
	("SEATRANS", 0, 8, 0, 0),
	("MSSLBOAT", 0, 8, 0, 0),
	("SEAMNLYR", 0, 8, 0, 0),
	("CARGOSHP", 0, 8, 0, 0),
	("FIGHTER", 0, 8, 0, 0),
	("BOMBER", 0, 8, 0, 0),
	("AIRTRANS", 0, 8, 0, 0),
	("AWAC", 0, 8, 8, 30),
	("JUGGRNT", 0, 8, 0, 0),
	("ALNTANK", 0, 8, 8, 8),
	("ALNASGUN", 0, 8, 0, 0),
	("ALNPLANE", 0, 8, 0, 0),
];

fn strip_for(tag: &str) -> Option<BaseUnitData> {
	let (_, bb, bc, tb, tc) = STRIPS.iter().find(|(t, ..)| *t == tag)?;
	Some(BaseUnitData {
		image_base: *bb,
		image_count: *bc,
		turret_image_base: *tb,
		turret_image_count: *tc,
		..Default::default()
	})
}

/// One placeable sprite: the body strip, its optional `S_*` shadow strip,
/// and the `D_*` record that says where body/turret frames live.
pub struct UnitEntry {
	pub tag: String,
	pub frames: Vec<IndexedFrame>,
	pub shadow: Vec<IndexedFrame>,
	pub data: BaseUnitData,
	/// Footprint in cells per side (1 for vehicles, 2 for big buildings),
	/// derived from the body sprite size — MAX.RES carries no flag for it.
	pub footprint: u32,
}

impl UnitEntry {
	pub fn body(&self) -> Option<&IndexedFrame> {
		self.frames.get(self.data.image_base as usize)
	}

	pub fn turret(&self) -> Option<&IndexedFrame> {
		if self.data.turret_image_count == 0 {
			return None;
		}
		self.frames.get(self.data.turret_image_base as usize)
	}

	/// Shadow strips mirror the body strip's indexing; clamp for the few
	/// sprites whose shadow has fewer frames.
	pub fn shadow_frame(&self) -> Option<&IndexedFrame> {
		if self.shadow.is_empty() {
			return None;
		}
		let i = (self.data.image_base as usize).min(self.shadow.len() - 1);
		self.shadow.get(i)
	}
}

pub struct UnitLibrary {
	pub units: Vec<UnitEntry>,
}

impl UnitLibrary {
	/// Load every unit/building sprite from `<max_path>/MAX.RES`. The roster
	/// is the set of tags with an `S_…` shadow companion (units and
	/// buildings cast shadows; FX/UI art doesn't) — RES tags are 8 bytes, so
	/// companion prefixes truncate the base to 6 chars (`S_AIRTRA` for
	/// `AIRTRANS`). Strip layout comes from the matching `D_…` template when
	/// one exists, else from the frame-count convention (8 chassis headings,
	/// then 8 turret headings). Sprites larger than [`MAX_SPRITE`] are
	/// skipped.
	pub fn load(max_path: &Path) -> Result<UnitLibrary, String> {
		let res = find_max_res(max_path)
			.ok_or_else(|| format!("MAX.RES not found in {} — check MaxPath", max_path.display()))?;
		let archive = read_res_index(&res).map_err(|e| format!("{}: {e}", res.display()))?;

		let has = |tag: &str| archive.entries.iter().any(|e| e.tag == tag);
		let short = |tag: &str| -> String { tag.chars().take(6).collect() };
		// Roster: the known table (canonical) plus any shadow-paired sprite
		// the table doesn't know (mod/edition extras).
		let mut tags: Vec<String> = STRIPS.iter().map(|(t, ..)| t.to_string()).filter(|t| has(t)).collect();
		for e in &archive.entries {
			let t = &e.tag;
			if t.chars().nth(1) != Some('_') && has(&format!("S_{}", short(t))) && !tags.iter().any(|k| k == t) {
				tags.push(t.clone());
			}
		}

		let mut units = Vec::new();
		for tag in tags {
			let Ok(Some(body)) = read_res_entry(&res, &tag) else { continue };
			let Ok(frames) = decode_multi_image_indexed(&body) else { continue };
			// Strip layout precedence: the per-unit table, the (per-class)
			// D_* template, frame-count inference.
			let data = strip_for(&tag)
				.or_else(|| match read_res_entry(&res, &format!("D_{}", short(&tag))) {
					Ok(Some(d)) => parse_base_unit_data(&d),
					_ => None,
				})
				.unwrap_or_else(|| infer_strips(frames.len()));
			let Some(first) = frames.get(data.image_base as usize) else { continue };
			if first.width > MAX_SPRITE || first.height > MAX_SPRITE {
				continue;
			}
			let shadow = match read_res_entry(&res, &format!("S_{}", short(&tag))) {
				Ok(Some(s)) => decode_multi_image_shadow_indexed(&s).unwrap_or_default(),
				_ => Vec::new(),
			};
			let footprint = if first.width > 64 || first.height > 64 { 2 } else { 1 };
			units.push(UnitEntry { tag, frames, shadow, data, footprint });
		}
		if units.is_empty() {
			return Err(format!("no unit sprites found in {}", res.display()));
		}
		units.sort_by(|a, b| a.tag.cmp(&b.tag));
		Ok(UnitLibrary { units })
	}

	pub fn find(&self, tag: &str) -> Option<usize> {
		let upper = tag.to_ascii_uppercase();
		self.units.iter().position(|u| u.tag == upper)
	}
}

/// Strip layout for sprites without a `D_…` template, from the format's
/// convention: vehicles pack 8 chassis headings first, turreted ones the 8
/// turret headings right after; buildings are single-frame (+ state frames).
fn infer_strips(frame_count: usize) -> BaseUnitData {
	let mut data = BaseUnitData { image_count: frame_count.min(255) as u8, ..Default::default() };
	if frame_count >= 16 {
		data.image_count = 8;
		data.turret_image_base = 8;
		data.turret_image_count = 8;
	} else if frame_count >= 8 {
		data.image_count = 8;
	}
	data
}

/// `MAX.RES`, tolerant of filename case (GOG/DOS installs differ).
fn find_max_res(dir: &Path) -> Option<std::path::PathBuf> {
	for name in ["MAX.RES", "max.res", "Max.res"] {
		let p = dir.join(name);
		if p.is_file() {
			return Some(p);
		}
	}
	// Last resort: scan the directory for any case mix.
	std::fs::read_dir(dir)
		.ok()?
		.flatten()
		.map(|e| e.path())
		.find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("MAX.RES")))
}

pub fn parse_team(s: &str) -> Option<u8> {
	if let Some(i) = TEAM_NAMES.iter().position(|n| *n == s) {
		return Some(i as u8);
	}
	s.parse::<u8>().ok().filter(|&n| (n as usize) < TEAMS)
}

// --- panel content (picker-style grid) --------------------------------------

const HEADER_H: f32 = 22.0;
const PAD: f32 = 4.0;
const GAP: f32 = 2.0;
const CELL: f32 = 52.0;
const SWATCH: f32 = 14.0;
pub const WHEEL_STEP: f32 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	Pick(usize),
	Team(u8),
	/// Toggle the unit-eraser tool.
	Eraser,
}

fn cols(body: Rect) -> usize {
	let inner = body.w - PAD * 2.0 - crate::ui::SCROLLBAR_W;
	(((inner + GAP) / (CELL + GAP)).floor() as usize).max(1)
}

fn item_rect(i: usize, body: Rect, scroll: f32) -> Rect {
	let n = cols(body);
	let (row, col) = (i / n, i % n);
	Rect::new(
		body.x + PAD + col as f32 * (CELL + GAP),
		body.y + HEADER_H + PAD - scroll + row as f32 * (CELL + GAP),
		CELL,
		CELL,
	)
}

pub fn max_scroll(count: usize, body: Rect) -> f32 {
	let rows = count.div_ceil(cols(body));
	let content = rows as f32 * (CELL + GAP) + PAD * 2.0 - GAP;
	crate::ui::scroll_max(content, body.h - HEADER_H)
}

fn swatch_rect(team: usize, body: Rect) -> Rect {
	Rect::new(body.x + PAD + team as f32 * (SWATCH + 4.0), body.y + (HEADER_H - SWATCH) / 2.0, SWATCH, SWATCH)
}

/// The "erase" toggle button, right of the team swatches.
fn eraser_rect(body: Rect) -> Rect {
	let swatches = swatch_rect(TEAMS - 1, body);
	Rect::new(swatches.x + swatches.w + 10.0, body.y + 3.0, 44.0, HEADER_H - 6.0)
}

pub fn scissor(body: Rect) -> Rect {
	Rect::new(body.x, body.y + HEADER_H, body.w, body.h - HEADER_H)
}

pub fn click(lib: Option<&UnitLibrary>, body: Rect, scroll: f32, x: f32, y: f32) -> Option<Action> {
	for team in 0..TEAMS {
		if swatch_rect(team, body).contains(x, y) {
			return Some(Action::Team(team as u8));
		}
	}
	if eraser_rect(body).contains(x, y) {
		return Some(Action::Eraser);
	}
	let lib = lib?;
	if y < body.y + HEADER_H {
		return None;
	}
	(0..lib.units.len()).find(|&i| item_rect(i, body, scroll).contains(x, y)).map(Action::Pick)
}

/// One thumbnail / overlay quad for the units GPU pass.
pub struct UnitQuad {
	pub rect: Rect,
	/// Atlas slot pixel origin of the sprite.
	pub origin: (u32, u32),
	pub sprite: (u32, u32),
	pub team: u8,
	pub shadow: bool,
}

pub struct View {
	pub quads: Vec<UnitQuad>,
	pub overlay: UiQuads,
	pub scissor: Rect,
}

/// Panel content: team swatch row + the unit grid. Thumbnails are body
/// frames only — shadows/turrets appear on the map placement, where they
/// matter for color judgement.
#[allow(clippy::too_many_arguments)]
pub fn view(
	lib: Option<&UnitLibrary>,
	slots: Option<&AtlasSlots>,
	active_unit: Option<usize>,
	team: u8,
	erasing: bool,
	scroll: f32,
	body: Rect,
	w: f32,
	h: f32,
	map: SteelMap,
	hot: Hot,
) -> View {
	let clip = scissor(body);
	let mut overlay = UiQuads::with_steel_map(map);
	let mut quads = Vec::new();

	// Header: steel strip, the five team swatches, the eraser toggle, and
	// the active tag.
	overlay.material(body.strip_top(HEADER_H), w, h, theme::TITLE);
	for t in 0..TEAMS {
		let r = swatch_rect(t, body);
		if t as u8 == team {
			overlay.rect(Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), w, h, theme::ACCENT);
		} else if hot.hover(r) {
			overlay.border(Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0), w, h, theme::INK_DIM);
		}
		overlay.rect(r, w, h, TEAM_SWATCH[t]);
	}
	// The eraser is a standard toggle key (green when active — toolbox parity).
	let er = eraser_rect(body);
	overlay.button_active(er, w, h, erasing, hot);
	overlay.label_in("erase", er, 6.0, crate::ui::FONT_SMALL, w, h, if erasing { theme::ACCENT } else { theme::INK });

	let (Some(lib), Some(slots)) = (lib, slots) else {
		// Word-wraps in a narrow dock instead of running off the panel.
		overlay.label_wrapped(
			"set MaxPath in config/mme.ini to load units",
			Rect::new(body.x, body.y + HEADER_H + 4.0, body.w, body.h - HEADER_H),
			PAD,
			crate::ui::FONT_SMALL,
			w,
			h,
			theme::INK_DIM,
		);
		return View { quads, overlay, scissor: clip };
	};

	let scroll = scroll.clamp(0.0, max_scroll(lib.units.len(), body));
	// Active tag, right-aligned — truncated when a narrow panel would run it
	// into the eraser button.
	let tag = active_unit.map(|i| lib.units[i].tag.as_str()).unwrap_or("");
	let avail = body.x + body.w - 6.0 - (er.x + er.w + 6.0);
	let tag = crate::text::fit_label(tag, crate::ui::FONT_SMALL, avail.max(0.0));
	let lx = body.x + body.w - 6.0 - crate::text::label_width(&tag, crate::ui::FONT_SMALL);
	overlay.label(&tag, lx, body.y + 4.0, crate::ui::FONT_SMALL, w, h, theme::INK);

	for (i, unit) in lib.units.iter().enumerate() {
		let r = item_rect(i, body, scroll);
		if r.y + r.h < clip.y || r.y > clip.y + clip.h {
			continue;
		}
		if active_unit == Some(i) {
			overlay.border(Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), w, h, theme::ACCENT);
		} else if hot.hover(r) && r.y >= clip.y {
			overlay.border(Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0), w, h, theme::INK_DIM);
		}
		thumbnail_quads(unit, slots.body(i), slots.turret(i), team, r, &mut quads);
	}

	let rows = lib.units.len().div_ceil(cols(body));
	overlay.scrollbar(clip, rows as f32 * (CELL + GAP) + 2.0 * PAD - GAP, scroll, w, h, hot);

	View { quads, overlay, scissor: clip }
}

/// Thumbnail quads for one grid cell: body + turret composited the way the
/// map does it — both hotspots anchored on the same point — then scaled to
/// fit the cell.
fn thumbnail_quads(
	unit: &UnitEntry,
	body_meta: Option<&crate::units_render::SlotMeta>,
	turret_meta: Option<&crate::units_render::SlotMeta>,
	team: u8,
	cell: Rect,
	quads: &mut Vec<UnitQuad>,
) {
	let Some(body_meta) = body_meta else { return };
	let Some(body) = unit.body() else { return };
	let turret = turret_meta.zip(unit.turret());

	// Bounding box of the composite, in sprite px relative to the anchor.
	let (mut x0, mut y0) = (-(body.hot_spot_x as f32), -(body.hot_spot_y as f32));
	let (mut x1, mut y1) = (x0 + body.width as f32, y0 + body.height as f32);
	if let Some((_, t)) = turret {
		let (tx0, ty0) = (-(t.hot_spot_x as f32), -(t.hot_spot_y as f32));
		x0 = x0.min(tx0);
		y0 = y0.min(ty0);
		x1 = x1.max(tx0 + t.width as f32);
		y1 = y1.max(ty0 + t.height as f32);
	}
	let scale = ((CELL - 4.0) / (x1 - x0).max(y1 - y0)).min(1.0);
	let (dw, dh) = ((x1 - x0) * scale, (y1 - y0) * scale);
	let (ox, oy) = (cell.x + (CELL - dw) / 2.0 - x0 * scale, cell.y + (CELL - dh) / 2.0 - y0 * scale);

	let place = |meta: &crate::units_render::SlotMeta, hot: (i32, i32)| UnitQuad {
		rect: Rect::new(
			ox - hot.0 as f32 * scale,
			oy - hot.1 as f32 * scale,
			meta.size.0 as f32 * scale,
			meta.size.1 as f32 * scale,
		),
		origin: meta.origin,
		sprite: meta.size,
		team,
		shadow: false,
	};
	quads.push(place(body_meta, (body.hot_spot_x, body.hot_spot_y)));
	if let Some((meta, t)) = turret {
		quads.push(place(meta, (t.hot_spot_x, t.hot_spot_y)));
	}
}

/// Build the map-overlay quads for the placed previews (the project's
/// `UnitNote` annotations): shadow quads first, then bodies, then turrets —
/// the game's compositing order. Notes whose tag isn't in the library (other
/// game edition, mod) are silently skipped.
pub fn map_quads(
	previews: &[map_core::UnitNote],
	lib: &UnitLibrary,
	slots: &AtlasSlots,
	pan: [f32; 2],
	zoom: f32,
) -> Vec<UnitQuad> {
	let mut shadows = Vec::new();
	let mut bodies = Vec::new();
	let mut turrets = Vec::new();

	for p in previews {
		let Some(index) = lib.find(&p.tag) else { continue };
		let unit = &lib.units[index];
		// The sprite hotspot lands on the footprint's center.
		let center =
			((p.x as f32 + unit.footprint as f32 / 2.0) * 64.0, (p.y as f32 + unit.footprint as f32 / 2.0) * 64.0);
		let quad = |meta: &crate::units_render::SlotMeta, hot: (i32, i32), shadow: bool| UnitQuad {
			rect: Rect::new(
				(center.0 - hot.0 as f32 - pan[0]) * zoom,
				(center.1 - hot.1 as f32 - pan[1]) * zoom,
				meta.size.0 as f32 * zoom,
				meta.size.1 as f32 * zoom,
			),
			origin: meta.origin,
			sprite: meta.size,
			team: p.team,
			shadow,
		};
		if let (Some(meta), Some(frame)) = (slots.shadow(index), unit.shadow_frame()) {
			shadows.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), true));
		}
		if let (Some(meta), Some(frame)) = (slots.body(index), unit.body()) {
			bodies.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), false));
		}
		if let (Some(meta), Some(frame)) = (slots.turret(index), unit.turret()) {
			turrets.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), false));
		}
	}

	shadows.append(&mut bodies);
	shadows.append(&mut turrets);
	shadows
}
