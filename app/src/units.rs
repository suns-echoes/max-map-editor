//! Unit sprite library + Units panel (palette-tuning aid): load unit and
//! building sprites from the user's own game data (`MaxPath`/MAX.RES),
//! list them in a picker-style grid, and stamp non-document *preview*
//! placements on the map so palette edits can be judged against real units.
//!
//! Pure logic - the GPU half (atlas + quad pass) lives in `units_render.rs`,
//! input routing in `main.rs`. Format knowledge (multi-image strips, `D_*`
//! base records, `S_*` shadow strips, team-color slots) follows re-MAX.

use std::path::Path;
use wgpu_ui::Vec2;

use max_assets::image::{IndexedFrame, decode_multi_image_indexed, decode_multi_image_shadow_indexed};
use max_assets::res::{read_res_entry, read_res_index};
use max_assets::units::{BaseUnitData, parse_base_unit_data};

use crate::theme;
use wgpu_ui::widget::{DrawCtx, EventCtx, LayoutCtx, Widget};
use wgpu_ui::{
	ArmFire, Button, ColorButton, CrossAlign, DrawList, Emboss, Event, Insets, Label, Length, Linear, PageKeys,
	Scroller, Size, TextAlign, TextRole, WidgetId, descendant, descendant_mut,
};

use crate::ui::Rect;
use crate::uikit_theme::rgba;
use crate::units_render::AtlasSlots;

/// The five player colors of the original game, in remap-table order.
pub const TEAMS: usize = 5;
pub const TEAM_NAMES: [&str; TEAMS] = ["red", "green", "blue", "gray", "yellow"];
/// Swatch colors for the team picker row (UI only - sprites recolor through
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
/// original game's base-unit-data - the `D_*` records in MAX.RES are shared
/// per-*class* templates, so per-unit truth has to come from a table like
/// this (fixed turrets keep their turret strip at frame 1, not 8; SCANNER /
/// AWAC turret strips spin through 16/30 frames; …). Explosion/projectile FX
/// are deliberately omitted - they aren't placeable map dressing.
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
	/// derived from the body sprite size - MAX.RES carries no flag for it.
	pub footprint: u32,
}

/// The most body/turret/shadow frames the editor ever atlases (and selects
/// between) per unit: the 8 compass headings (`UNIT_ANGLE_*`, N..NW). Vehicles
/// and soldiers rotate through these; ground cover uses the same slot to hold
/// its random decorative variant (LRGSLAB's 5 textures, etc.); animation-heavy
/// sprites (INFANTRY/COMMANDO, 100+ walk frames) only ever need the 8 resting
/// headings. So capping at 8 covers every selectable frame while keeping the
/// atlas small (per M.A.X. Port research, see SAVE-EDITOR.md S2.2).
pub const MAX_HEADINGS: usize = 8;

/// A connector host carries eight strut frames (one per connector half-edge)
/// starting at `connector_image_base`.
pub const CONNECTOR_FRAMES: usize = 8;

/// Each connector half-edge bit (`enums.hpp`) → its strut frame offset from
/// `connector_image_base`, transcribed from `UnitInfo::RenderWithConnectors`
/// (`unitinfo.cpp`): NL/ET/SL/WT are the four base struts (offsets 0..3), and
/// NR/EB/SR/WB (a 2×2's second half-edge per side) sit at 4..7.
pub const CONNECTOR_BIT_FRAME: [(u16, usize); CONNECTOR_FRAMES] = [
	(0x01, 0), // NL
	(0x04, 1), // ET
	(0x10, 2), // SL
	(0x40, 3), // WT
	(0x02, 4), // NR
	(0x08, 5), // EB
	(0x20, 6), // SR
	(0x80, 7), // WB
];

impl UnitEntry {
	/// How many selectable body frames this unit exposes (compass headings or
	/// decorative variants), capped at [`MAX_HEADINGS`] and by the frames that
	/// actually decoded. Always ≥ 1.
	pub fn body_count(&self) -> usize {
		let base = self.data.image_base as usize;
		let avail = self.frames.len().saturating_sub(base);
		(self.data.image_count as usize).min(MAX_HEADINGS).min(avail).max(1)
	}

	/// The body frame for selection index `h` (a heading / variant), clamped to
	/// [`Self::body_count`]. `h = 0` is heading N / variant 0.
	pub fn body_frame(&self, h: usize) -> Option<&IndexedFrame> {
		let h = h.min(self.body_count() - 1);
		self.frames.get(self.data.image_base as usize + h)
	}

	pub fn body(&self) -> Option<&IndexedFrame> {
		self.body_frame(0)
	}

	/// How many selectable turret frames (0 when this unit has no turret).
	pub fn turret_count(&self) -> usize {
		if self.data.turret_image_count == 0 {
			return 0;
		}
		let base = self.data.turret_image_base as usize;
		let avail = self.frames.len().saturating_sub(base);
		(self.data.turret_image_count as usize).min(MAX_HEADINGS).min(avail)
	}

	/// The turret frame for selection index `h`, clamped; `None` for a
	/// turret-less unit.
	pub fn turret_frame(&self, h: usize) -> Option<&IndexedFrame> {
		let count = self.turret_count();
		if count == 0 {
			return None;
		}
		let h = h.min(count - 1);
		self.frames.get(self.data.turret_image_base as usize + h)
	}

	pub fn turret(&self) -> Option<&IndexedFrame> {
		self.turret_frame(0)
	}

	/// Which turret frame to draw for an object whose stored `turret_angle` may be
	/// independent of its body `body_angle`: the turret heading when it indexes a
	/// real turret frame for this unit, else the body heading. Non-turret units
	/// store engine scratch in `turret_angle` (0..255), and the engine deploys a
	/// turret facing the body, so both cases correctly fall back to the body.
	pub fn turret_index_for(&self, turret_angle: u8, body_angle: u8) -> usize {
		if (turret_angle as usize) < self.turret_count() { turret_angle as usize } else { body_angle as usize }
	}

	/// How many connector-strut frames this unit exposes (0 for non-hosts). Hosts
	/// carry 8 - one per connector half-edge - starting at `connector_image_base`.
	pub fn connector_count(&self) -> usize {
		if self.data.connector_image_count == 0 {
			return 0;
		}
		let base = self.data.connector_image_base as usize;
		let avail = self.frames.len().saturating_sub(base);
		(self.data.connector_image_count as usize).min(CONNECTOR_FRAMES).min(avail)
	}

	/// The connector-strut frame at side offset `k` (0..8, see [`CONNECTOR_BIT_FRAME`]);
	/// `None` when the unit is not a connector host or the frame wasn't decoded.
	pub fn connector_frame(&self, k: usize) -> Option<&IndexedFrame> {
		if k >= self.connector_count() {
			return None;
		}
		self.frames.get(self.data.connector_image_base as usize + k)
	}

	/// How many selectable shadow frames (0 when the unit casts none), capped.
	pub fn shadow_count(&self) -> usize {
		self.shadow.len().min(MAX_HEADINGS)
	}

	/// The shadow frame for selection index `h`. Shadow strips mirror the body
	/// strip's indexing; clamp for the sprites whose shadow has fewer frames.
	pub fn shadow_frame_at(&self, h: usize) -> Option<&IndexedFrame> {
		if self.shadow.is_empty() {
			return None;
		}
		self.shadow.get(h.min(self.shadow.len() - 1))
	}
}

pub struct UnitLibrary {
	pub units: Vec<UnitEntry>,
	/// Roster index per physical unit type id, built once at load — the O(1)
	/// answer to "which sprite is type `ty`" the per-frame paths (footprint
	/// frames, hit-testing, previews) ask, instead of an
	/// O(roster) name scan with a `to_ascii_uppercase` allocation each time.
	/// `None` for a type with no sprite in this MAX.RES.
	by_type: [Option<u16>; max_assets::save::UNIT_END],
}

impl UnitLibrary {
	/// Wraps a loaded roster, building the type-id index. Every construction
	/// goes through here (tests included) so `find_type` never disagrees with
	/// `find`.
	pub fn new(units: Vec<UnitEntry>) -> UnitLibrary {
		let mut by_type = [None; max_assets::save::UNIT_END];
		for (i, unit) in units.iter().enumerate() {
			if let Some(ty) = max_assets::save::unit_type_id(&unit.tag) {
				by_type[usize::from(ty)] = Some(i as u16);
			}
		}
		UnitLibrary { units, by_type }
	}

	/// The roster index for physical type `unit_type`, O(1) off the load-time
	/// table; `None` for an unknown type or one with no sprite.
	pub fn find_type(&self, unit_type: u16) -> Option<usize> {
		self.by_type.get(usize::from(unit_type)).copied().flatten().map(usize::from)
	}

	/// Load every unit/building sprite from `<max_path>/MAX.RES`. The roster
	/// is the set of tags with an `S_…` shadow companion (units and
	/// buildings cast shadows; FX/UI art doesn't) - RES tags are 8 bytes, so
	/// companion prefixes truncate the base to 6 chars (`S_AIRTRA` for
	/// `AIRTRANS`). Strip layout comes from the matching `D_…` template when
	/// one exists, else from the frame-count convention (8 chassis headings,
	/// then 8 turret headings). Sprites larger than [`MAX_SPRITE`] are
	/// skipped.
	pub fn load(max_path: &Path) -> Result<UnitLibrary, String> {
		let res = find_max_res(max_path)
			.ok_or_else(|| format!("MAX.RES not found in {} - check MaxPath", max_path.display()))?;
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
			// The `D_*` template (byte 0..8) is the authoritative frame layout;
			// the per-unit STRIPS table is a curated override for body/turret
			// where the template is missing or wrong.
			let d_star = match read_res_entry(&res, &format!("D_{}", short(&tag))) {
				Ok(Some(d)) => parse_base_unit_data(&d),
				_ => None,
			};
			// Strip layout precedence: the per-unit table, the D_* template,
			// frame-count inference.
			let mut data = strip_for(&tag).or(d_star).unwrap_or_else(|| infer_strips(frames.len()));
			let Some(first) = frames.get(data.image_base as usize) else { continue };
			if first.width > MAX_SPRITE || first.height > MAX_SPRITE {
				continue;
			}
			let footprint = if first.width > 64 || first.height > 64 { 2 } else { 1 };
			// STRIPS / inference carry no connector geometry, so a connector host
			// (CNCT_4W, buildings, fixed turrets) would lose its strut frame base.
			// Backfill it - the map overlay draws struts from these frames - from the
			// unit's own D_<name> template, else the shared class template the engine
			// assigns (D_LRGBLD / D_SMLBLD / D_FIXED). Struts sit right after the
			// body/turret/firing frames at connector_image_base in the unit's sheet.
			if data.connector_image_count == 0 {
				let source: Option<BaseUnitData> = d_star.filter(|d| d.connector_image_count != 0).or_else(|| {
					let name = connector_class_template(&tag, footprint, data.turret_image_count != 0)?;
					read_res_entry(&res, name).ok().flatten().and_then(|d| parse_base_unit_data(&d))
				});
				if let Some(d) = source {
					data.connector_image_base = d.connector_image_base;
					data.connector_image_count = d.connector_image_count;
				}
			}
			let shadow = match read_res_entry(&res, &format!("S_{}", short(&tag))) {
				Ok(Some(s)) => decode_multi_image_shadow_indexed(&s).unwrap_or_default(),
				_ => Vec::new(),
			};
			units.push(UnitEntry { tag, frames, shadow, data, footprint });
		}
		if units.is_empty() {
			return Err(format!("no unit sprites found in {}", res.display()));
		}
		units.sort_by(|a, b| a.tag.cmp(&b.tag));
		Ok(UnitLibrary::new(units))
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

/// The shared connector-strut *class* template a connector host draws its struts
/// from when it has no own `D_<name>` record: the engine assigns `D_LRGBLD` to
/// 2×2 buildings, `D_SMLBLD` to 1×1 buildings / CNCT_4W, and `D_FIXED` to fixed
/// turrets. `None` for anything that isn't a connector host (mobile units,
/// ground cover), so only real hosts ever gain struts.
fn connector_class_template(tag: &str, footprint: u32, has_turret: bool) -> Option<&'static str> {
	let ty = max_assets::save::unit_type_id(tag)?;
	if !max_assets::save::is_connector_host_type(ty) {
		return None;
	}
	Some(if has_turret {
		"D_FIXED"
	} else if footprint == 2 {
		"D_LRGBLD"
	} else {
		"D_SMLBLD"
	})
}

/// `MAX.RES`, tolerant of filename case (GOG/DOS installs differ). Shared with
/// the resource-marker loader (`markers.rs`).
pub(crate) fn find_max_res(dir: &Path) -> Option<std::path::PathBuf> {
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

// --- panel (a real widget tree: header row over the sprite grid) -------------

/// The header band's height. Unlike the tiles / templates explorers this is a
/// **declared constant**, not a flowed height: the band holds one row of
/// controls whatever the dock does, so the grid below it — and the scissor the
/// native sprite pass gets — never move (U5.7).
const HEADER_H: f32 = 22.0;
/// Inner padding of the sprite grid.
const PAD: f32 = 4.0;
/// Gap between cells and between rows.
const GAP: f32 = 2.0;
/// Cell (thumbnail) edge length.
const CELL: f32 = 52.0;
/// The colour fill of a team swatch...
const SWATCH: f32 = 12.0;
/// ...and the padding between that fill and its themed face — the ring the
/// hand-drawn selection backing used to occupy. 3px (1 up from the widget's
/// default): a wider ring, so the active team's face reads at a glance. The
/// fill gives up the pixel instead of the face growing, keeping the bank's
/// 90px tiling.
const SWATCH_INSET: f32 = 3.0;
/// A swatch key: the fill plus its ring, so five of them tile the same 90px the
/// old hand-placed row did.
const SWATCH_FACE: f32 = SWATCH + 2.0 * SWATCH_INSET;
/// The "erase" toggle, right of the swatch bank.
const ERASER_W: f32 = 44.0;
const ERASER_H: f32 = HEADER_H - 6.0;
/// The header row's margins: the swatch bank starts 2px in (its faces carry the
/// old 4px inset), and the tag readout ends 6px from the right edge.
const HDR_PAD: Insets = Insets { left: 2.0, top: 0.0, right: 6.0, bottom: 0.0 };
/// The gap between the three parts of the header — swatch bank, eraser, tag.
const HDR_GAP: f32 = 8.0;

/// What a fired action tag resolved to. `Pick` carries the index into the
/// loaded [`UnitLibrary`]; the shell resolves it against live state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
	Pick(usize),
	Team(u8),
	/// Toggle the unit-eraser tool.
	Eraser,
}

/// The tag space: a kind in the high bits over a 32-bit payload, so one
/// `Ui::actions` poll answers for the whole panel (U5.4's shape). Kind `0` is
/// deliberately unused — a stray zero tag resolves to nothing.
const KIND_SHIFT: u32 = 32;
/// A team swatch: the payload is its index into [`TEAM_NAMES`].
const KIND_TEAM: u64 = 1;
/// The eraser toggle (no payload).
const KIND_ERASER: u64 = 2;
/// A sprite pick: the payload is its index into the library.
const KIND_PICK: u64 = 3;

const fn tag(kind: u64, i: usize) -> u64 {
	(kind << KIND_SHIFT) | i as u64
}

/// The units action a fired tag stands for, or `None` if it is not one of this
/// panel's (the shell polls every tag its `Ui` collected).
pub fn action_of(tag: u64) -> Option<Action> {
	let i = (tag & 0xffff_ffff) as usize;
	match tag >> KIND_SHIFT {
		KIND_TEAM => (i < TEAMS).then_some(Action::Team(i as u8)),
		KIND_ERASER => Some(Action::Eraser),
		// The roster is the shell's (it holds the library), so its range is too.
		KIND_PICK => Some(Action::Pick(i)),
		_ => None,
	}
}

/// The cell under `p` in `grid`, scrolled to `offset`, out of `count` sprites —
/// the grid's domain hit oracle. The padding, the gaps between cells and the run
/// past the last sprite belong to nobody, exactly as the panel's old `click`
/// oracle had them.
///
/// Free rather than a method so [`UnitsGrid`] can hand it to its own `ArmFire`
/// without borrowing itself immutably and mutably at once.
fn cell_at(grid: &crate::cellgrid::Grid, offset: f32, count: usize, p: Vec2) -> Option<usize> {
	if !grid.body.contains(p) {
		return None;
	}
	let i = grid.index_at(p.x, p.y, offset)?;
	(i < count && grid.item_rect(i, offset).contains(p)).then_some(i)
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

/// The units-panel state the chrome reflects, snapshotted into [`UnitsContent`]
/// each frame so the retained draw holds no library/atlas borrow. The native
/// sprite thumbnails are built separately ([`quads`]) from the live library.
#[derive(Clone)]
pub struct Snapshot {
	team: u8,
	erasing: bool,
	active_unit: Option<usize>,
	/// Unit count (0 when no library is loaded) — drives rings + scrollbar.
	count: usize,
	/// The selected unit's tag, right-aligned in the header.
	active_tag: Option<String>,
	/// Whether the library + atlas are loaded (else the "set MaxPath" message).
	loaded: bool,
}

impl Snapshot {
	/// Snapshot the units-panel state for one frame's chrome draw.
	pub fn of(
		lib: Option<&UnitLibrary>,
		slots: Option<&AtlasSlots>,
		active_unit: Option<usize>,
		team: u8,
		erasing: bool,
	) -> Self {
		Self {
			team,
			erasing,
			active_unit,
			count: lib.map(|l| l.units.len()).unwrap_or(0),
			active_tag: lib.and_then(|l| active_unit.and_then(|i| l.units.get(i)).map(|u| u.tag.clone())),
			loaded: lib.is_some() && slots.is_some(),
		}
	}

	fn empty() -> Self {
		Self { team: 0, erasing: false, active_unit: None, count: 0, active_tag: None, loaded: false }
	}
}

/// The unit-grid thumbnail quads (native sprite pass), built shell-side because
/// they need the library + atlas slots. `cells` is the visible window the grid
/// widget reports ([`UnitsContent::visible_cells`]), so the quads ride exactly
/// the geometry the rings and the wells do — there is one computation of it, and
/// nothing to drift (the U5.3 invariant). Thumbnails are body frames only —
/// shadows/turrets appear on the map placement, where they matter for color
/// judgement.
pub fn quads(
	lib: Option<&UnitLibrary>,
	slots: Option<&AtlasSlots>,
	team: u8,
	cells: &[(usize, Rect)],
) -> Vec<UnitQuad> {
	let mut quads = Vec::new();
	let (Some(lib), Some(slots)) = (lib, slots) else {
		return quads;
	};
	for &(i, r) in cells {
		let Some(unit) = lib.units.get(i) else { continue };
		// Thumbnails show the resting heading (frame 0).
		thumbnail_quads(unit, slots.body(i, 0), slots.turret(i, 0), team, r, &mut quads);
	}
	quads
}

/// Black wells behind each visible grid cell: the unit sprites paint onto black
/// *per cell* (not one panel-wide backdrop, and not the steel panel), so a
/// sprite's palette reads against a neutral ground. Drawn **before** the native
/// sprite pass (the thumbnails composite on top) and clamped to `clip` — the
/// grid widget's own rect, which is also the pass's scissor — so a cell
/// scrolled under the header never paints over it.
pub fn cell_backgrounds(dl: &mut DrawList, cells: &[(usize, Rect)], clip: Rect) {
	for &(_, r) in cells {
		let top = r.y.max(clip.y);
		let bot = (r.y + r.h).min(clip.y + clip.h);
		if bot <= top {
			continue;
		}
		dl.fill_rect(Rect::new(r.x, top, r.w, bot - top), rgba(theme::SPRITE_WELL));
	}
}

/// The units panel's **content widget**: the scrolling sprite grid.
///
/// It owns exactly what §5.2 allows a content widget to own — the
/// [`crate::cellgrid::Grid`] geometry, its own [`Scroller`], the domain cell
/// pick, the selection/hover rings, and the visible window the native sprite
/// pass renders the thumbnails from — and no chrome: the five team swatches,
/// the eraser and the tag readout are its **siblings** in the panel tree, never
/// its children.
///
/// Arranged straight *into* its viewport (not into a tall content rect), which
/// is what keeps G7 deferred: the widget clips its own draw, scrolls the rows
/// through it, and hands the GPU pass a scissor that is simply its own rect.
pub struct UnitsGrid {
	id: WidgetId,
	snap: Snapshot,
	rect: Rect,
	scroller: Scroller,
	/// The theme's scrollbar metric, sampled at `arrange` — the gutter
	/// [`grid`](Self::grid) reserves, kept equal to the bar the `Scroller`
	/// paints.
	gutter: f32,
	/// Arm-on-press / fire-on-release-inside over the cells — the domain hit
	/// test a content widget keeps (the panel's chrome oracle is gone).
	clicks: ArmFire<usize>,
	/// The cell the pointer is over, tracked here because a *cell* is this
	/// widget's own domain — the `Ui` can only say whether the grid is hovered.
	/// The ring is drawn only while it agrees (see [`Self::draw`]).
	hover: Option<usize>,
}

impl UnitsGrid {
	fn new() -> Self {
		Self {
			id: wgpu_ui::next_id(),
			snap: Snapshot::empty(),
			rect: Rect::ZERO,
			scroller: Scroller::new(),
			gutter: 8.0,
			clicks: ArmFire::new(),
			hover: None,
		}
	}

	/// The cell geometry over this widget's own arranged rect. The grid *is* the
	/// viewport now, so it carries no header offset — the header band is the
	/// sibling above it.
	fn grid(&self) -> crate::cellgrid::Grid {
		crate::cellgrid::Grid { body: self.rect, cell: CELL, gap: GAP, pad: PAD, gutter: self.gutter, row_extra: 0.0 }
	}

	/// Sprite `i`'s cell rect at the current scroll.
	fn item_rect(&self, i: usize) -> Rect {
		self.grid().item_rect(i, self.scroller.offset())
	}

	/// The cell under `p`, if any — the domain hit oracle.
	fn cell_at(&self, p: Vec2) -> Option<usize> {
		cell_at(&self.grid(), self.scroller.offset(), self.snap.count, p)
	}

	/// Every cell touching the window, as `(index, rect)`. The shell builds both
	/// the black wells and the native sprite quads from this one list, so the
	/// three layers of the grid — wells, sprites, rings — are laid out once
	/// (U5.3's invariant; here the borrow allows reading it back off the widget
	/// rather than recomputing it).
	fn visible_cells(&self) -> Vec<(usize, Rect)> {
		(0..self.snap.count)
			.map(|i| (i, self.item_rect(i)))
			.filter(|(_, r)| r.bottom() >= self.rect.y && r.y <= self.rect.bottom())
			.collect()
	}
}

impl Widget for UnitsGrid {
	fn measure(&mut self, avail: Size, _ctx: &mut LayoutCtx) -> Size {
		avail
	}

	fn arrange(&mut self, rect: Rect, ctx: &mut LayoutCtx) {
		self.rect = rect;
		self.gutter = ctx.theme.metrics().scrollbar;
		// The grid window is the viewport; the flowed rows are the content.
		self.scroller.layout(ctx, rect, self.grid().content_height(self.snap.count));
	}

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		if !ctx.is_base() {
			return;
		}
		dl.push_clip(self.rect);
		if !self.snap.loaded {
			// The "set MaxPath" note lives in the clipped grid layer so it can't
			// spill past a short panel (the templates explorer's empty state).
			ctx.theme.text_wrapped(
				dl,
				ctx.fonts,
				Rect::new(self.rect.x, self.rect.y + 4.0, self.rect.w, self.rect.h),
				PAD,
				"set MaxPath in resources/user/config/mme.ini to load units",
				TextRole::Small,
				Emboss::Engraved,
				rgba(theme::INK_DIM),
			);
			dl.pop_clip();
			return;
		}
		// The thumbnails themselves are the native sprite pass (see [`quads`]);
		// this `DrawList` carries only the rings over them. A cell rings dim under
		// the pointer, gated on the `Ui` agreeing that this widget is hovered at
		// all — which anything owning the pointer above it makes false.
		let hovered = ctx.is_hovered(self.id).then_some(self.hover).flatten();
		for (i, r) in self.visible_cells() {
			let ring = Rect::new(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0);
			if self.snap.active_unit == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::ACCENT));
			} else if hovered == Some(i) {
				dl.stroke_rect(ring, 1.0, rgba(theme::INK_DIM));
			}
		}
		dl.pop_clip();
		// The bar sits over the rows, outside their clip.
		self.scroller.draw(dl, ctx);
	}

	fn event(&mut self, ev: &Event, ctx: &mut EventCtx) -> bool {
		// Track the cell under the pointer while this widget is the one being
		// pointed at; anything else (the cursor leaving, another widget taking
		// the pointer) drops it.
		match ev {
			Event::PointerMoved { .. } | Event::PointerButton { .. } => {
				self.hover = ctx.is_target(self.id).then(|| self.cell_at(ctx.pointer)).flatten();
			}
			Event::PointerLeft | Event::Focus(false) => self.hover = None,
			_ => {}
		}
		let (grid, offset, count) = (self.grid(), self.scroller.offset(), self.snap.count);
		let handled = self.clicks.event(ev, ctx, self.id, |p| cell_at(&grid, offset, count, p));
		// The fire goes out as an action tag, so the shell polls one place for the
		// whole panel (the header controls' `Ui::actions`) instead of a second
		// channel per widget kind.
		if let Some(i) = self.clicks.take_outcome() {
			ctx.fire(self.id, Some(tag(KIND_PICK, i)));
		}
		if handled {
			return true;
		}
		// The cells keep first refusal; the wheel, the bar and the paging keys
		// fall to the scroller.
		self.scroller.event_with(ev, ctx, self.id, PageKeys::WhenHovered)
	}

	fn rect(&self) -> Rect {
		self.rect
	}

	fn id(&self) -> WidgetId {
		self.id
	}

	/// The cells and the scrollbar column claim the pointer; the padding and the
	/// gaps between them stay inert, exactly as the old `click` oracle had them.
	/// The bar has to be claimed explicitly — [`Scroller`] only takes a press
	/// when its owner is the dispatch target (U5.5).
	fn hit_test(&self, pos: Vec2) -> Option<WidgetId> {
		let bar = self.scroller.has_bar() && self.scroller.track_rect().contains(pos);
		(bar || self.cell_at(pos).is_some()).then_some(self.id)
	}
}

/// The units panel as a retained `wgpu_ui` [`Widget`]: a thin root over a
/// `Linear` column of the header row and the [`UnitsGrid`]. It exists to hold
/// the id tables and to push the per-frame [`Snapshot`] into them; everything
/// else — layout, paint, hover, arming, firing, scrolling — is the tree's.
///
/// The header is a fixed-height row rather than a flow: a bank of five
/// [`wgpu_ui::ColorButton`] team keys (G8), the eraser [`wgpu_ui::Button`], and
/// the active sprite's tag as a right-aligned, ellipsizing [`Label`] (G18) in
/// the leftover space.
pub struct UnitsContent {
	id: WidgetId,
	root: Linear,
	/// The five team swatches, in [`TEAM_NAMES`] order.
	swatches: [WidgetId; TEAMS],
	/// The eraser toggle.
	eraser: WidgetId,
	/// The active sprite's tag readout.
	tag: WidgetId,
	grid: WidgetId,
	rect: Rect,
}

impl Default for UnitsContent {
	fn default() -> Self {
		Self::new()
	}
}

impl UnitsContent {
	pub fn new() -> Self {
		// The swatch faces tile with no gap: each carries its own `SWATCH_INSET`
		// ring, which is exactly the spacing the hand-placed row used to leave
		// between fills (and the room the selection backing needed).
		let mut bank = Linear::row();
		let mut swatches = [WidgetId::NONE; TEAMS];
		for (t, slot) in swatches.iter_mut().enumerate() {
			let key = ColorButton::new(rgba(TEAM_SWATCH[t]), SWATCH_FACE, SWATCH_FACE)
				.inset(SWATCH_INSET)
				.action(tag(KIND_TEAM, t));
			*slot = key.id();
			bank = bank.push(key);
		}
		// The eraser is a standard toggle key (lit when the tool is armed —
		// toolbox parity).
		let eraser = Button::new("erase").small().sized(ERASER_W, ERASER_H).action(tag(KIND_ERASER, 0));
		let eraser_id = eraser.id();
		// The tag takes the leftover width and sits at its right edge, cut to a
		// `...` tail rather than running back over the eraser in a narrow dock.
		let tag = Label::new("").small().align(TextAlign::Right).ellipsize().with_id();
		let tag_id = tag.id();
		let header = Linear::row()
			.padding(HDR_PAD)
			.spacing(HDR_GAP)
			.cross_align(CrossAlign::Center)
			.push(bank)
			.push(eraser)
			.child(tag, Length::Flex(1.0));

		let grid = UnitsGrid::new();
		let grid_id = grid.id();
		// The band is a **constant**, so — unlike the flowed headers — the grid's
		// rect (and the native pass's scissor) is the same every frame.
		// `Stretch` is what gives the header row the panel's full width: a
		// `Linear` measures to its *content*, so without it the row would be as
		// wide as its three parts and the tag would stop wherever they ended (a
		// `Wrap` header measures to the available width, which is why the tiles /
		// templates columns need no such line).
		let root = Linear::column()
			.cross_align(CrossAlign::Stretch)
			.child(header, Length::Fixed(HEADER_H))
			.child(grid, Length::Flex(1.0));
		Self { id: wgpu_ui::next_id(), root, swatches, eraser: eraser_id, tag: tag_id, grid: grid_id, rect: Rect::ZERO }
	}

	/// Push one frame's state into the retained tree: the selected team, the
	/// eraser's weight, the tag readout, and the grid's sprites.
	pub fn sync(&mut self, snap: Snapshot) {
		for (t, &id) in self.swatches.iter().enumerate() {
			if let Some(key) = descendant_mut::<ColorButton>(&mut self.root, id) {
				key.set_selected(t as u8 == snap.team);
			}
		}
		if let Some(key) = descendant_mut::<Button>(&mut self.root, self.eraser) {
			key.set_selected(snap.erasing);
		}
		if let Some(label) = descendant_mut::<Label>(&mut self.root, self.tag) {
			// Only a loaded panel has a roster to name a sprite from.
			label.set_text(if snap.loaded { snap.active_tag.clone().unwrap_or_default() } else { String::new() });
		}
		if let Some(grid) = descendant_mut::<UnitsGrid>(&mut self.root, self.grid) {
			grid.snap = snap;
		}
	}

	/// The visible sprite cells as `(index, rect)` plus the scissor to clip them
	/// to — the shell draws the black wells and then the native sprite pass over
	/// them, under this panel's chrome. Read *after* `build`, which is what
	/// settles both the grid's rect and the scroll offset they hang off.
	pub fn visible_cells(&self) -> (Vec<(usize, Rect)>, Rect) {
		descendant::<UnitsGrid>(&self.root, self.grid)
			.map_or_else(|| (Vec::new(), Rect::ZERO), |g| (g.visible_cells(), g.rect))
	}
}

impl Widget for UnitsContent {
	// The plain `event` forward: every child commits on release-inside (there
	// is no `Select` here, and nothing else that fires on the press), so the
	// shell polls this panel once, after the release dispatch.
	crate::panel_ui::thin_root_plumbing!(arrange, event);

	fn draw(&self, dl: &mut DrawList, ctx: &DrawCtx) {
		// The header's steel band, under the tree.
		if ctx.is_base()
			&& let Some(band) = Widget::child(&self.root, 0).map(Widget::rect)
		{
			ctx.theme.header_band(dl, band);
		}
		self.root.draw(dl, ctx);
	}
}

/// Thumbnail quads for one grid cell: body + turret composited the way the
/// map does it - both hotspots anchored on the same point - then scaled to
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

/// Build the map-overlay quads for the project's placed [`map_core::MapObject`]s
/// (preview annotations on an ordinary map, or an opened save's units / slabs /
/// rubble): shadow quads first, then bodies, then turrets - the game's
/// compositing order. Each object's `unit_type` (a `ResourceID`) bridges to a
/// sprite by its canonical name; objects with no matching sprite (FX / internal
/// types, another edition, a mod) are silently skipped.
/// The map's objects in **paint order**, each with its index: ground cover
/// first (slabs, rubble, roads), then everything else, both in list order.
///
/// This is the one definition of what is under what. A slab is a floor, so it
/// belongs under the building standing on it however the two were laid down —
/// the same split the game keeps by holding ground cover in its own unit list.
/// The sprite pass draws in this order and every hit test scans its reverse
/// ([`crate::state::EditorState::objects_at`]), so what a click picks is always
/// what is visibly on top.
pub fn draw_order(objects: &[map_core::MapObject]) -> impl DoubleEndedIterator<Item = (usize, &map_core::MapObject)> {
	let cover = |o: &map_core::MapObject| max_assets::save::is_ground_cover_type(o.unit_type);
	objects
		.iter()
		.enumerate()
		.filter(move |(_, o)| cover(o))
		.chain(objects.iter().enumerate().filter(move |(_, o)| !cover(o)))
}

pub fn object_quads(
	objects: &[map_core::MapObject],
	lib: &UnitLibrary,
	slots: &AtlasSlots,
	pan: [f32; 2],
	zoom: f32,
) -> Vec<UnitQuad> {
	let placements = draw_order(objects).filter_map(|(_, o)| {
		Some(Placement {
			index: lib.find_type(o.unit_type)?,
			x: o.x as f32,
			y: o.y as f32,
			team: o.team,
			angle: o.props.angle,
			turret_angle: o.props.turret_angle,
			connectors: o.props.connectors,
		})
	});
	unit_quads_from(placements, lib, slots, pan, zoom)
}

/// One object to lay out: its roster `index` (resolved O(1) through
/// [`UnitLibrary::find_type`]), its footprint's top-left cell, owning `team`,
/// the body `angle` (heading / decorative variant), and the independent
/// `turret_angle`. A turret unit whose stored `turret_angle` isn't a valid
/// frame index (engine scratch on non-deployed units) falls back to the body
/// heading.
struct Placement {
	index: usize,
	x: f32,
	y: f32,
	team: u8,
	angle: u8,
	turret_angle: u8,
	/// Connector adjacency bitmask; one strut sprite is drawn per set half-edge
	/// bit (`CONNECTOR_BIT_FRAME`). Non-zero only for connector hosts.
	connectors: u16,
}

/// One body-sprite quad for `object`, scaled to fit `target` (logical px) and
/// centred — the connector grid's footprint thumbnail (S4.4, Unit Properties
/// panel). Body frame only (no shadow / turret), the resting frame, tinted to
/// the object's team. `None` when the type has no sprite. Drawn through the
/// units pass with `scale` + a panel scissor, like the units-panel thumbnails.
pub fn object_sprite_quad(
	object: &map_core::MapObject,
	target: Rect,
	lib: &UnitLibrary,
	slots: &AtlasSlots,
) -> Option<UnitQuad> {
	let index = lib.find_type(object.unit_type)?;
	let meta = slots.body(index, 0)?; // the resting body frame
	let (sw, sh) = (meta.size.0 as f32, meta.size.1 as f32);
	if sw <= 0.0 || sh <= 0.0 {
		return None;
	}
	// Fit-scale into the target, preserving aspect, and centre.
	let s = (target.w / sw).min(target.h / sh);
	let (w, h) = (sw * s, sh * s);
	let rect = Rect::new(target.x + (target.w - w) * 0.5, target.y + (target.h - h) * 0.5, w, h);
	Some(UnitQuad { rect, origin: meta.origin, sprite: meta.size, team: object.team, shadow: false })
}

/// The full composited sprite for `object` — body + connector struts + turret, at
/// its current heading / turret angle / connector mask — scaled to fit `target`
/// and centred, tinted to the object's team. This is the live preview at the top
/// of the Unit Properties panel (item 11): the connections read as real struts,
/// exactly the way the game draws them (item 4b). Empty when the type has no
/// sprite. Unlike the map pass, no shadow (the strut strip has none anyway) and
/// the whole composite is fit as one unit so struts never overflow the well.
pub fn object_preview_quads(
	object: &map_core::MapObject,
	target: Rect,
	lib: &UnitLibrary,
	slots: &AtlasSlots,
) -> Vec<UnitQuad> {
	let Some(index) = lib.find_type(object.unit_type) else {
		return Vec::new();
	};
	let unit = &lib.units[index];
	let h = object.props.angle as usize;
	let th = unit.turret_index_for(object.props.turret_angle, object.props.angle);
	// Body, then one strut per set half-edge, then the turret — in draw order (each
	// carries its own hotspot, so they compose relative to the footprint centre).
	let mut parts: Vec<(&crate::units_render::SlotMeta, (i32, i32))> = Vec::new();
	if let (Some(meta), Some(frame)) = (slots.body(index, h), unit.body_frame(h)) {
		parts.push((meta, (frame.hot_spot_x, frame.hot_spot_y)));
	}
	if object.props.connectors != 0 {
		for (bit, k) in CONNECTOR_BIT_FRAME {
			if object.props.connectors & bit == 0 {
				continue;
			}
			if let (Some(meta), Some(frame)) = (slots.connector(index, k), unit.connector_frame(k)) {
				parts.push((meta, (frame.hot_spot_x, frame.hot_spot_y)));
			}
		}
	}
	if let (Some(meta), Some(frame)) = (slots.turret(index, th), unit.turret_frame(th)) {
		parts.push((meta, (frame.hot_spot_x, frame.hot_spot_y)));
	}
	if parts.is_empty() {
		return Vec::new();
	}
	// Union bounds in local sprite space (a part's top-left is `-hotspot`), then a
	// single fit-scale of the whole composite into the target.
	let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
	for (meta, hot) in &parts {
		let (x0, y0) = (-(hot.0 as f32), -(hot.1 as f32));
		min_x = min_x.min(x0);
		min_y = min_y.min(y0);
		max_x = max_x.max(x0 + meta.size.0 as f32);
		max_y = max_y.max(y0 + meta.size.1 as f32);
	}
	let (uw, uh) = (max_x - min_x, max_y - min_y);
	if uw <= 0.0 || uh <= 0.0 {
		return Vec::new();
	}
	let s = (target.w / uw).min(target.h / uh);
	let (ox, oy) = (target.x + (target.w - uw * s) * 0.5, target.y + (target.h - uh * s) * 0.5);
	parts
		.iter()
		.map(|(meta, hot)| {
			let (x0, y0) = (-(hot.0 as f32), -(hot.1 as f32));
			UnitQuad {
				rect: Rect::new(
					ox + (x0 - min_x) * s,
					oy + (y0 - min_y) * s,
					meta.size.0 as f32 * s,
					meta.size.1 as f32 * s,
				),
				origin: meta.origin,
				sprite: meta.size,
				team: object.team,
				shadow: false,
			}
		})
		.collect()
}

/// Shared quad builder for the project's placed objects. Each [`Placement`] is
/// resolved to a sprite, the right frame is picked for its body heading, its
/// hotspot is centred on the footprint, and it is split into shadow / body /
/// turret quads (all shadows sorted beneath all bodies). The turret uses the
/// object's independent `turret_angle` when that is a real frame index for this
/// unit, else follows the body heading (the engine deploys turrets facing the
/// body, and non-turret units store scratch there - see SAVE-EDITOR.md S4.4).
fn unit_quads_from(
	placements: impl Iterator<Item = Placement>,
	lib: &UnitLibrary,
	slots: &AtlasSlots,
	pan: [f32; 2],
	zoom: f32,
) -> Vec<UnitQuad> {
	let mut shadows = Vec::new();
	let mut bodies = Vec::new();
	let mut turrets = Vec::new();

	for p in placements {
		let index = p.index;
		let unit = &lib.units[index];
		let h = p.angle as usize;
		// The turret faces its own stored heading; a value past the unit's turret
		// frames (engine scratch on units the editor doesn't turret) reverts to
		// the body heading, matching the previous "turret follows body" behaviour.
		let th = unit.turret_index_for(p.turret_angle, p.angle);
		// The sprite hotspot lands on the footprint's center.
		let center = ((p.x + unit.footprint as f32 / 2.0) * 64.0, (p.y + unit.footprint as f32 / 2.0) * 64.0);
		let team = p.team;
		let quad = |meta: &crate::units_render::SlotMeta, hot: (i32, i32), shadow: bool| UnitQuad {
			rect: Rect::new(
				(center.0 - hot.0 as f32 - pan[0]) * zoom,
				(center.1 - hot.1 as f32 - pan[1]) * zoom,
				meta.size.0 as f32 * zoom,
				meta.size.1 as f32 * zoom,
			),
			origin: meta.origin,
			sprite: meta.size,
			team,
			shadow,
		};
		if let (Some(meta), Some(frame)) = (slots.shadow(index, h), unit.shadow_frame_at(h)) {
			shadows.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), true));
		}
		if let (Some(meta), Some(frame)) = (slots.body(index, h), unit.body_frame(h)) {
			bodies.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), false));
		}
		// Connector struts: one sprite per set half-edge bit, from the host's own
		// strut strip. Each frame carries its own hotspot (drawn at the footprint
		// centre like the body), so a strut reaches toward its neighbour. They
		// composite in the body layer (beneath turrets); strut shadows are omitted
		// (the shadow strip has no strut frames - an editor-overlay simplification).
		if p.connectors != 0 {
			for (bit, k) in CONNECTOR_BIT_FRAME {
				if p.connectors & bit == 0 {
					continue;
				}
				if let (Some(meta), Some(frame)) = (slots.connector(index, k), unit.connector_frame(k)) {
					bodies.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), false));
				}
			}
		}
		if let (Some(meta), Some(frame)) = (slots.turret(index, th), unit.turret_frame(th)) {
			turrets.push(quad(meta, (frame.hot_spot_x, frame.hot_spot_y), false));
		}
	}

	shadows.append(&mut bodies);
	shadows.append(&mut turrets);
	shadows
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::uikit_menu::MenuChrome;
	use wgpu_ui::{DrawCmd, Modifiers, PointerButton, ScrollDelta, Ui, widget::DrawPass};

	/// A `w`×`h` indexed frame whose `width` doubles as an identity marker.
	fn frame(w: u32, h: u32, hx: i32, hy: i32) -> IndexedFrame {
		IndexedFrame { width: w, height: h, hot_spot_x: hx, hot_spot_y: hy, pixels: vec![0u8; (w * h) as usize] }
	}

	/// An atlas placement (all `SlotMeta` fields are public).
	fn meta(origin: (u32, u32), size: (u32, u32)) -> crate::units_render::SlotMeta {
		crate::units_render::SlotMeta { origin, size }
	}

	fn unit(tag: &str, data: BaseUnitData, frames: Vec<IndexedFrame>, shadow: Vec<IndexedFrame>) -> UnitEntry {
		UnitEntry { tag: tag.to_string(), frames, shadow, data, footprint: 1 }
	}

	/// A minimal in-memory roster (no disk / RES access).
	fn lib_of(tags: &[&str]) -> UnitLibrary {
		let units =
			tags.iter().map(|t| unit(t, BaseUnitData::default(), vec![frame(8, 8, 4, 4)], Vec::new())).collect();
		UnitLibrary::new(units)
	}

	/// A loaded snapshot (library + atlas present) - `Snapshot::of` can't report
	/// `loaded` without a GPU-built `AtlasSlots`, so build it directly.
	fn loaded_snap(team: u8, erasing: bool, active_unit: Option<usize>, count: usize, tag: Option<&str>) -> Snapshot {
		Snapshot { team, erasing, active_unit, count, active_tag: tag.map(str::to_string), loaded: true }
	}

	/// The chrome fixture + the panel hosted in a `Ui`, laid out into `body`.
	/// A stock `Button` measures its own label, so this needs the real fonts
	/// (`Fonts::new()` + `Gunmetal` panics with "FontId(0) is not registered").
	fn hosted(body: Rect, snap: Snapshot) -> (MenuChrome, Ui, WidgetId) {
		let (_device, _queue, chrome) = crate::visual_test::chrome_fixture();
		let content = UnitsContent::new();
		let id = content.id();
		let mut ui = Ui::new(content);
		ui.get_mut::<UnitsContent>(id).expect("typed root").sync(snap);
		ui.layout_in(body, chrome.theme(), chrome.fonts());
		(chrome, ui, id)
	}

	fn press(pressed: bool, at: Vec2) -> Event {
		Event::PointerButton { button: PointerButton::Primary, pressed, pos: at, mods: Modifiers::NONE }
	}

	/// The grid child, borrowed typed off the hosted tree.
	fn grid_of(ui: &Ui, id: WidgetId) -> &UnitsGrid {
		let content = ui.get::<UnitsContent>(id).expect("typed root");
		descendant::<UnitsGrid>(&content.root, content.grid).expect("the content widget")
	}

	/// The header row (the root column's first child).
	fn header_of(ui: &Ui, id: WidgetId) -> &dyn Widget {
		Widget::child(&ui.get::<UnitsContent>(id).expect("typed root").root, 0).expect("the header")
	}

	/// The arranged rect of team swatch `t`'s themed face.
	fn swatch_face(ui: &Ui, id: WidgetId, t: usize) -> Rect {
		let sw = ui.get::<UnitsContent>(id).expect("typed root").swatches[t];
		ui.rect_of(sw).expect("the swatch is arranged")
	}

	/// The base-pass draw of the hosted panel.
	fn drawn(chrome: &MenuChrome, ui: &Ui) -> DrawList {
		let mut dl = DrawList::new();
		ui.draw_pass(&mut dl, chrome.theme(), chrome.fonts(), DrawPass::Base);
		dl
	}

	/// `parse_team` accepts the five color names and the numeric team indices
	/// `0..TEAMS`, and rejects unknown names and out-of-range numbers.
	#[test]
	fn parse_team_maps_names_and_indices() {
		assert_eq!(parse_team("red"), Some(0));
		assert_eq!(parse_team("yellow"), Some(4));
		assert_eq!(parse_team("0"), Some(0));
		assert_eq!(parse_team("3"), Some(3));
		assert_eq!(parse_team("5"), None, "index past the roster is rejected");
		assert_eq!(parse_team("purple"), None, "an unknown color is rejected");
		assert_eq!(parse_team(""), None);
	}

	/// `strip_for` returns the per-unit table row for a known tag and `None`
	/// otherwise; `infer_strips` follows the frame-count convention (8 chassis
	/// headings, then 8 turret headings once a strip is big enough).
	#[test]
	fn strip_for_table_and_infer_strips_convention() {
		let tank = strip_for("TANK").expect("TANK is in the table");
		assert_eq!(
			(tank.image_base, tank.image_count, tank.turret_image_base, tank.turret_image_count),
			(0, 8, 8, 8),
			"a turreted vehicle: 8 body + 8 turret headings"
		);
		let turret = strip_for("GUNTURRT").expect("GUNTURRT is in the table");
		assert_eq!(
			(turret.image_base, turret.image_count, turret.turret_image_base, turret.turret_image_count),
			(0, 1, 1, 8),
			"a fixed turret: 1 body frame + an 8-heading turret"
		);
		assert!(strip_for("NOSUCHUNIT").is_none(), "an unknown tag has no table row");

		// Frame-count inference for sprites without a table/template row.
		let big = infer_strips(20);
		assert_eq!(
			(big.image_count, big.turret_image_base, big.turret_image_count),
			(8, 8, 8),
			">=16 -> body + turret"
		);
		let vehicle = infer_strips(8);
		assert_eq!((vehicle.image_count, vehicle.turret_image_count), (8, 0), "8..16 -> 8 body, no turret");
		let building = infer_strips(2);
		assert_eq!((building.image_count, building.turret_image_count), (2, 0), "<8 -> all frames body");
	}

	/// The header band is a **declared constant**, whatever the dock width does,
	/// and the grid is exactly its complement — which is what makes the grid's
	/// rect (and so the native sprite pass's scissor) the same every frame. The
	/// five swatch keys and the eraser sit inside the band, the swatch fills
	/// where the hand-placed row always put them.
	#[test]
	fn the_header_band_is_a_constant_and_the_grid_is_its_complement() {
		for w in [300.0, 220.0, 700.0] {
			let body = Rect::new(10.0, 20.0, w, 400.0);
			let (_chrome, ui, id) = hosted(body, loaded_snap(0, false, None, 40, Some("TANK")));
			let (band, grid) = (header_of(&ui, id).rect(), grid_of(&ui, id).rect);
			assert_eq!(band.h, HEADER_H, "a {w}px dock keeps the one-row band");
			assert_eq!(grid.y, band.bottom(), "the grid starts below the band");
			assert_eq!(grid.bottom(), body.bottom(), "and reaches the body bottom");
			assert_eq!((grid.x, grid.w), (body.x, body.w));

			let s0 = swatch_face(&ui, id, 0);
			assert_eq!((s0.w, s0.h), (SWATCH_FACE, SWATCH_FACE));
			assert_eq!(
				(s0.x, s0.y),
				(body.x + HDR_PAD.left, body.y + (HEADER_H - SWATCH_FACE) / 2.0),
				"the face starts at the band margin, centred in the band"
			);
			assert_eq!(
				swatch_face(&ui, id, 1).x,
				s0.x + SWATCH_FACE,
				"the faces tile - each carries its own ring, so the fills keep their old pitch"
			);
			let last = swatch_face(&ui, id, TEAMS - 1);
			let eraser = ui.get::<UnitsContent>(id).expect("typed root").eraser;
			let er = ui.rect_of(eraser).expect("the eraser is arranged");
			assert!(er.x >= last.right(), "the eraser sits right of the last swatch");
			assert_eq!((er.w, er.h), (ERASER_W, ERASER_H));
			for r in [s0, last, er] {
				assert!(r.y >= band.y && r.bottom() <= band.bottom(), "{r:?} sits inside the band {band:?}");
			}
		}
	}

	/// The grid flows cells left-to-right inside its own rect, wraps to the next
	/// row at the column count, widens with the panel, and lifts by the scroll
	/// offset.
	#[test]
	fn the_grid_flows_its_cells_and_scrolls_them() {
		let snap = loaded_snap(0, false, None, 60, None);
		let cols = |w: f32| {
			let (_chrome, ui, id) = hosted(Rect::new(0.0, 0.0, w, 400.0), snap.clone());
			grid_of(&ui, id).grid().cols()
		};
		assert!(cols(480.0) > cols(120.0), "a wider panel fits more columns");

		let body = Rect::new(10.0, 20.0, 300.0, 400.0);
		let (chrome, mut ui, id) = hosted(body, snap);
		let n = grid_of(&ui, id).grid().cols();
		let (r0, r1, wrap) =
			(grid_of(&ui, id).item_rect(0), grid_of(&ui, id).item_rect(1), grid_of(&ui, id).item_rect(n));
		let window = grid_of(&ui, id).rect;
		assert_eq!((r0.w, r0.h), (CELL, CELL));
		assert_eq!((r0.x, r0.y), (window.x + PAD, window.y + PAD), "cell 0 sits a pad inside the grid window");
		assert_eq!((r1.x, r1.y), (r0.x + CELL + GAP, r0.y), "cells flow along the row");
		assert_eq!((wrap.x, wrap.y), (r0.x, r0.y + CELL + GAP), "the next row wraps to column 0, one pitch down");

		// The wheel over the grid lifts every cell by the offset it took.
		let wheel = Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)),
			pos: window.center(),
			mods: Modifiers::NONE,
		};
		assert!(ui.dispatch(&[wheel]).wants_pointer(), "the grid takes the wheel");
		let scrolled = grid_of(&ui, id).scroller.offset();
		assert!(scrolled > 0.0, "one wheel notch scrolls");
		assert_eq!(grid_of(&ui, id).item_rect(n).y, wrap.y - scrolled, "scrolling lifts every cell");

		// A roster that fits its panel never scrolls at all.
		let tall = Rect::new(0.0, 0.0, 300.0, 2000.0);
		let (_chrome, mut ui, id) = hosted(tall, loaded_snap(0, false, None, 3, None));
		let wheel =
			Event::Scroll { delta: ScrollDelta::Lines(Vec2::new(0.0, 1.0)), pos: tall.center(), mods: Modifiers::NONE };
		ui.dispatch(&[wheel]);
		assert_eq!(grid_of(&ui, id).scroller.offset(), 0.0, "a few units never scroll");
		drop(chrome);
	}

	/// **The window the three grid layers share.** The sprites are a native GPU
	/// pass, so the content widget's job is to report the visible cells and the
	/// scissor to clip them to — and that scissor is exactly its own rect, which
	/// the header band and the panel body bracket. The black wells are built from
	/// the same list, so the wells, the sprites and the rings cannot drift.
	#[test]
	fn the_grid_reports_the_visible_window_and_a_scissor_that_is_its_own_rect() {
		let body = Rect::new(10.0, 20.0, 300.0, 300.0);
		let count = 60;
		let (_chrome, ui, id) = hosted(body, loaded_snap(0, false, None, count, None));
		let (cells, clip) = ui.get::<UnitsContent>(id).expect("typed root").visible_cells();

		let grid = grid_of(&ui, id);
		assert_eq!(clip, grid.rect, "the scissor is the grid's viewport");
		assert_eq!(clip.y, header_of(&ui, id).rect().bottom(), "which starts below the header band");
		assert_eq!(clip.bottom(), body.bottom(), "and reaches the panel's bottom");
		assert!(cells.len() < count, "off-window rows are culled");
		assert!(!cells.is_empty());
		for &(i, r) in &cells {
			assert_eq!(r, grid.item_rect(i), "each cell carries its own rect");
			assert!(r.bottom() >= clip.y && r.y <= clip.bottom(), "and touches the window");
		}

		// The wells: one black rect per visible cell, clamped into the scissor
		// band so a partially-scrolled cell never paints over the header.
		let mut dl = DrawList::new();
		cell_backgrounds(&mut dl, &cells, clip);
		let black = rgba(theme::SPRITE_WELL);
		let wells: Vec<_> = dl
			.cmds
			.iter()
			.filter_map(|c| match c {
				DrawCmd::Solid { rect, color } if *color == black => Some(*rect),
				_ => None,
			})
			.collect();
		assert_eq!(wells.len(), cells.len(), "one black well per visible cell");
		for (w, &(i, r)) in wells.iter().zip(&cells) {
			assert_eq!((w.x, w.w), (r.x, r.w), "well {i} aligns to its cell horizontally");
			assert!(w.y >= clip.y - 0.01, "well {i} never rises above the header");
			assert!(w.bottom() <= clip.bottom() + 0.01, "well {i} stays inside the scissor band");
		}
		// A no-library panel has no cells, so no wells.
		let (_chrome, ui, id) = hosted(body, Snapshot::empty());
		let (none, clip) = ui.get::<UnitsContent>(id).expect("typed root").visible_cells();
		let mut empty = DrawList::new();
		cell_backgrounds(&mut empty, &none, clip);
		assert!(empty.cmds.is_empty(), "no units -> no wells");
	}

	/// Every header control fires **its own** tag on a press + release-inside:
	/// the five team swatches their team, the eraser its toggle. That is the
	/// whole click path now — no hit oracle, no panel-wide `ArmFire`, and no
	/// action written down twice. Empty header space stays inert.
	#[test]
	fn every_header_control_fires_its_own_tag() {
		let body = Rect::new(10.0, 20.0, 300.0, 400.0);
		let (_chrome, mut ui, id) = hosted(body, loaded_snap(0, false, None, 12, None));

		for t in 0..TEAMS {
			let at = swatch_face(&ui, id, t).center();
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "swatch {t}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(ui.actions().len(), 1, "swatch {t}: one key, one action");
			assert_eq!(action_of(ui.actions()[0]), Some(Action::Team(t as u8)), "swatch {t}");
		}

		let eraser = ui.get::<UnitsContent>(id).expect("typed root").eraser;
		let at = ui.rect_of(eraser).expect("arranged").center();
		ui.dispatch(&[press(true, at)]);
		ui.dispatch(&[press(false, at)]);
		assert_eq!(action_of(ui.actions()[0]), Some(Action::Eraser));

		// The empty stretch of header the tag readout sits in claims nothing.
		let empty = Vec2::new(body.right() - 3.0, body.y + 3.0);
		assert!(!ui.dispatch(&[press(true, empty)]).wants_pointer(), "empty header falls through");
		assert!(ui.actions().is_empty());
	}

	/// Every tag resolves back to what built it, and a tag from nowhere resolves
	/// to nothing — the mapping the shell runs a fired action through.
	#[test]
	fn every_tag_resolves_to_its_own_action() {
		for t in 0..TEAMS {
			assert_eq!(action_of(tag(KIND_TEAM, t)), Some(Action::Team(t as u8)), "team {t}");
		}
		assert_eq!(action_of(tag(KIND_TEAM, TEAMS)), None, "a team past the roster is nothing");
		assert_eq!(action_of(tag(KIND_ERASER, 0)), Some(Action::Eraser));
		assert_eq!(action_of(tag(KIND_PICK, 7)), Some(Action::Pick(7)));
		assert_eq!(action_of(0), None, "the unused kind resolves to nothing");
	}

	/// A sprite arms on the press and fires its roster index on the release, at
	/// rest and scrolled; the gaps between cells and the run past the last sprite
	/// belong to nobody — exactly as the old `click` oracle had them.
	#[test]
	fn a_sprite_picks_and_the_gaps_do_not() {
		let body = Rect::new(10.0, 20.0, 300.0, 300.0);
		let count = 60;
		let (_chrome, mut ui, id) = hosted(body, loaded_snap(0, false, None, count, None));

		for i in [0usize, 1, 5, 12] {
			let at = grid_of(&ui, id).item_rect(i).center();
			ui.dispatch(&[press(true, at)]);
			assert!(ui.actions().is_empty(), "cell {i}: a press only arms");
			ui.dispatch(&[press(false, at)]);
			assert_eq!(action_of(ui.actions()[0]), Some(Action::Pick(i)), "cell {i}");
		}

		// Scrolled, every visible cell still round-trips at its drawn rect.
		let wheel = Event::Scroll {
			delta: ScrollDelta::Lines(Vec2::new(0.0, 2.0)),
			pos: grid_of(&ui, id).rect.center(),
			mods: Modifiers::NONE,
		};
		ui.dispatch(&[wheel]);
		let (cells, window) = ui.get::<UnitsContent>(id).expect("typed root").visible_cells();
		let whole: Vec<_> = cells.into_iter().filter(|(_, r)| r.y >= window.y).collect();
		for (i, r) in whole {
			ui.dispatch(&[press(true, r.center()), press(false, r.center())]);
			assert_eq!(action_of(ui.actions()[0]), Some(Action::Pick(i)), "scrolled cell {i}");
		}

		// The gap between two cells in a row is inert, and consumes nothing.
		let r0 = grid_of(&ui, id).item_rect(0);
		let gap = Vec2::new(r0.right() + GAP / 2.0, r0.center().y);
		assert_eq!(grid_of(&ui, id).cell_at(gap), None, "the gap between cells picks nothing");
		let resp = ui.dispatch(&[press(true, gap), press(false, gap)]);
		assert!(!resp.wants_pointer(), "and consumes nothing");
		assert!(ui.actions().is_empty());

		// A cell past the loaded count is empty space, whatever the grid geometry.
		let (_chrome, mut ui, id) = hosted(body, loaded_snap(0, false, None, 12, None));
		let past = grid_of(&ui, id).item_rect(12).center();
		assert!(!ui.dispatch(&[press(true, past)]).wants_pointer(), "a cell past the count is empty");
	}

	/// `UnitEntry` multi-frame accessors: `body_frame(h)` / `turret_frame(h)`
	/// index the strip at `image_base + h` / `turret_image_base + h`, each capped
	/// at `MAX_HEADINGS` and by the frames that exist; `shadow_frame_at(h)`
	/// mirrors the heading, clamped to the shadow strip length.
	#[test]
	fn unit_entry_multi_frame_accessors() {
		// 8 body frames (widths 10..18) at image_base 2, plus a 4-frame turret.
		let frames: Vec<IndexedFrame> = (0..12).map(|i| frame(10 + i, 10, 0, 0)).collect();
		let data = BaseUnitData {
			image_base: 2,
			image_count: 8,
			turret_image_base: 8,
			turret_image_count: 4,
			..Default::default()
		};
		let u = unit("X", data, frames, Vec::new());
		assert_eq!(u.body_count(), 8, "8 heading frames available");
		assert_eq!(u.body().map(|f| f.width), Some(12), "body() = heading 0 at image_base");
		assert_eq!(u.body_frame(3).map(|f| f.width), Some(15), "heading 3 = image_base + 3");
		assert_eq!(u.body_frame(99).map(|f| f.width), Some(19), "an over-range heading clamps to the last");
		assert_eq!(u.turret_count(), 4, "4 turret frames");
		assert_eq!(u.turret_frame(1).map(|f| f.width), Some(19), "turret heading 1 = turret_image_base + 1");
		// An independent turret_angle within the 4 turret frames is honoured; a
		// value past them (or engine scratch on a non-turret unit) uses the body.
		assert_eq!(u.turret_index_for(2, 5), 2, "in-range turret_angle draws that turret frame");
		assert_eq!(u.turret_index_for(6, 5), 5, "turret_angle past the turret frames falls back to the body");
		assert_eq!(u.turret_index_for(255, 3), 3, "engine-scratch turret_angle falls back to the body heading");
	}

	/// `connector_frame(k)` indexes the strut strip at `connector_image_base + k`
	/// (0..count), and `CONNECTOR_BIT_FRAME` maps the eight half-edge bits onto a
	/// unique 0..8 strut offset in the engine's `RenderWithConnectors` order.
	#[test]
	fn connector_frame_accessor_and_bit_table() {
		// 8 strut frames (widths 40..47) at connector_image_base 3.
		let frames: Vec<IndexedFrame> = (0..11).map(|i| frame(40 + i, 10, 0, 0)).collect();
		let data = BaseUnitData { connector_image_base: 3, connector_image_count: 8, ..Default::default() };
		let u = unit("C", data, frames, Vec::new());
		assert_eq!(u.connector_count(), 8, "8 strut frames");
		assert_eq!(u.connector_frame(0).map(|f| f.width), Some(43), "strut 0 = connector_image_base");
		assert_eq!(u.connector_frame(7).map(|f| f.width), Some(50), "strut 7 = connector_image_base + 7");
		assert!(u.connector_frame(8).is_none(), "past the strut count = no frame");
		let none = unit("N", BaseUnitData::default(), vec![frame(9, 9, 0, 0)], Vec::new());
		assert_eq!(none.connector_count(), 0, "connector_image_count 0 -> not a host");
		assert!(none.connector_frame(0).is_none());

		// The eight bits map to eight distinct offsets covering 0..8.
		let mut offsets: Vec<usize> = CONNECTOR_BIT_FRAME.iter().map(|&(_, k)| k).collect();
		offsets.sort();
		assert_eq!(offsets, (0..CONNECTOR_FRAMES).collect::<Vec<_>>(), "0..8 covered once each");
		let bits: Vec<u16> = CONNECTOR_BIT_FRAME.iter().map(|&(b, _)| b).collect();
		assert_eq!(bits.iter().fold(0u16, |a, b| a | b), 0xFF, "the eight bits are the full mask");

		// image_count over the cap is limited to MAX_HEADINGS (infantry walk cycle).
		let walk = unit(
			"I",
			BaseUnitData { image_count: 200, ..Default::default() },
			(0..200).map(|i| frame(1 + (i % 90) as u32, 10, 0, 0)).collect(),
			vec![],
		);
		assert_eq!(walk.body_count(), MAX_HEADINGS, "walk-heavy strips cap at the 8 resting headings");

		let hidden =
			unit("Y", BaseUnitData { turret_image_count: 0, ..Default::default() }, vec![frame(9, 9, 0, 0)], vec![]);
		assert_eq!(hidden.turret_count(), 0, "turret_image_count 0 -> no turret");
		assert!(hidden.turret_frame(0).is_none(), "turret_image_count 0 -> no turret frame");

		let shadow: Vec<IndexedFrame> = (0..4).map(|i| frame(20 + i, 20, 0, 0)).collect();
		let z = unit("Z", BaseUnitData::default(), vec![frame(9, 9, 0, 0)], shadow);
		assert_eq!(z.shadow_count(), 4, "4 shadow frames");
		assert_eq!(z.shadow_frame_at(2).map(|f| f.width), Some(22), "shadow follows the heading");
		assert_eq!(z.shadow_frame_at(10).map(|f| f.width), Some(23), "an over-range heading clamps to the last shadow");
		let none = unit("N", BaseUnitData::default(), vec![frame(9, 9, 0, 0)], Vec::new());
		assert!(none.shadow_frame_at(0).is_none(), "no shadow strip -> none");
	}

	/// `UnitLibrary::find` matches tags case-insensitively and misses unknowns.
	#[test]
	fn library_find_is_case_insensitive() {
		let lib = lib_of(&["TANK", "SCOUT", "AWAC"]);
		assert_eq!(lib.find("tank"), Some(0));
		assert_eq!(lib.find("Awac"), Some(2));
		assert_eq!(lib.find("nope"), None);
	}

	/// `Snapshot::of` reports the roster count, the selected unit's tag (only
	/// when the index is in range), and `loaded` only when both a library and an
	/// atlas are present.
	#[test]
	fn snapshot_of_reflects_library_and_selection() {
		let empty = Snapshot::of(None, None, None, 0, false);
		assert_eq!(empty.count, 0);
		assert!(!empty.loaded && empty.active_tag.is_none());

		let lib = lib_of(&["ALPHA", "BETA", "GAMMA"]);
		let s = Snapshot::of(Some(&lib), None, Some(1), 2, true);
		assert_eq!(s.count, 3, "count follows the library");
		assert_eq!((s.team, s.erasing), (2, true));
		assert_eq!(s.active_tag.as_deref(), Some("BETA"), "the active tag is the selected unit's");
		assert!(!s.loaded, "no atlas slots -> not loaded");

		let oob = Snapshot::of(Some(&lib), None, Some(99), 0, false);
		assert!(oob.active_tag.is_none(), "an out-of-range selection has no tag");
		assert_eq!(oob.count, 3);
	}

	/// A panel with no library paints its five team swatches and explains itself
	/// in the clipped grid layer; a loaded one drops the note. The swatch keys
	/// paint their team colors either way — the roster is what is missing, not
	/// the team choice.
	#[test]
	fn an_unloaded_panel_keeps_its_swatches_and_explains_itself() {
		let body = Rect::new(0.0, 0.0, 260.0, 400.0);
		let glyphs = |dl: &DrawList| dl.cmds.iter().filter(|c| matches!(c, DrawCmd::Glyph { .. })).count();

		let (chrome, ui, id) = hosted(body, Snapshot::empty());
		let un = drawn(&chrome, &ui);
		let (chrome2, loaded_ui, _) = hosted(body, loaded_snap(0, false, None, 0, None));
		let loaded = drawn(&chrome2, &loaded_ui);
		assert!(glyphs(&un) > glyphs(&loaded), "the no-library note only draws when unloaded");

		for t in 0..TEAMS {
			let want = rgba(TEAM_SWATCH[t]);
			let fill = swatch_face(&ui, id, t).inset(wgpu_ui::Insets::all(SWATCH_INSET));
			assert!(
				un.cmds.iter().any(|c| matches!(c, DrawCmd::Solid { rect, color } if *rect == fill && *color == want)),
				"swatch {t} paints its color"
			);
		}
	}

	/// The selected team's swatch and the armed eraser read **selected** — the
	/// theme's own latched face, where the panel used to hand-paint an accent
	/// backing and a green label. Moving the selection moves it.
	#[test]
	fn the_selected_team_and_the_armed_eraser_read_selected() {
		let body = Rect::new(0.0, 0.0, 260.0, 400.0);
		let selected = |ui: &Ui, id: WidgetId| {
			ui.get::<UnitsContent>(id)
				.expect("typed root")
				.swatches
				.map(|s| ui.get::<ColorButton>(s).expect("a swatch").selected())
		};
		let (_chrome, mut ui, id) = hosted(body, loaded_snap(2, false, None, 0, None));
		assert_eq!(selected(&ui, id), [false, false, true, false, false], "team 2 is the chosen swatch");

		ui.get_mut::<UnitsContent>(id).expect("typed root").sync(loaded_snap(0, true, None, 0, None));
		assert_eq!(selected(&ui, id), [true, false, false, false, false], "the selection moves with the team");

		let eraser = ui.get::<UnitsContent>(id).expect("typed root").eraser;
		assert!(ui.get::<Button>(eraser).expect("the eraser").selected(), "an armed eraser reads selected");
		ui.get_mut::<UnitsContent>(id).expect("typed root").sync(loaded_snap(0, false, None, 0, None));
		assert!(!ui.get::<Button>(eraser).expect("the eraser").selected(), "and an idle one does not");
	}

	/// The tag readout names the armed sprite, right-aligned at the end of the
	/// header row and cut to a `...` tail rather than running back over the
	/// eraser in a narrow dock (G18). An unloaded panel names nothing.
	#[test]
	fn the_tag_readout_names_the_armed_sprite() {
		let body = Rect::new(0.0, 0.0, 260.0, 400.0);
		let text = |ui: &Ui, id: WidgetId| {
			let content = ui.get::<UnitsContent>(id).expect("typed root");
			descendant::<Label>(&content.root, content.tag).expect("the tag").text().to_string()
		};
		let (_chrome, ui, id) = hosted(body, loaded_snap(0, false, Some(3), 12, Some("BATTLSHP")));
		assert_eq!(text(&ui, id), "BATTLSHP");
		let tag_id = ui.get::<UnitsContent>(id).expect("typed root").tag;
		let slot = ui.rect_of(tag_id).expect("the readout is arranged");
		let eraser = ui.get::<UnitsContent>(id).expect("typed root").eraser;
		assert!(slot.x >= ui.rect_of(eraser).expect("arranged").right(), "the readout takes the leftover width");
		assert_eq!(slot.right(), body.right() - HDR_PAD.right, "ending a margin short of the panel edge");

		let (_chrome, ui, id) = hosted(body, Snapshot::empty());
		assert_eq!(text(&ui, id), "", "no library -> nothing to name");
	}

	/// The armed sprite rings in accent and a hovered cell rings dim; a ring
	/// scrolled out of the window is culled with its cell.
	#[test]
	fn the_grid_rings_the_armed_sprite_and_the_hovered_one() {
		let tall = Rect::new(0.0, 0.0, 260.0, 900.0);
		let (chrome, mut ui, id) = hosted(tall, loaded_snap(0, false, Some(0), 40, None));
		let ringed = |ui: &Ui, cell: Rect, color| {
			drawn(&chrome, ui).cmds.iter().any(|c| match c {
				DrawCmd::Solid { rect, color: c } => {
					rect.x == cell.x - 1.0 && rect.y == cell.y - 1.0 && *c == rgba(color)
				}
				_ => false,
			})
		};
		let (cell0, cell1) = (grid_of(&ui, id).item_rect(0), grid_of(&ui, id).item_rect(1));
		assert!(ringed(&ui, cell0, theme::ACCENT), "the armed sprite keeps its selection ring");
		assert!(!ringed(&ui, cell1, theme::INK_DIM), "nothing is hovered at rest");

		ui.dispatch(&[Event::PointerMoved { pos: cell1.center() }]);
		assert_eq!(grid_of(&ui, id).hover, Some(1), "the grid knows which cell it is");
		assert!(ringed(&ui, cell1, theme::INK_DIM), "the hovered sprite rings dimly");

		// A selection past the window is culled with its cell.
		let short = Rect::new(0.0, 0.0, 260.0, 120.0);
		let (chrome, ui, id) = hosted(short, loaded_snap(0, false, Some(39), 40, None));
		let culled = grid_of(&ui, id).item_rect(39);
		let ring_at = drawn(&chrome, &ui).cmds.iter().any(|c| match c {
			DrawCmd::Solid { rect, color } => rect.y == culled.y - 1.0 && *color == rgba(theme::ACCENT),
			_ => false,
		});
		assert!(!ring_at, "a selection past the clip window draws no ring");
	}

	/// A press in the scrollbar column pages — the bar is chrome the grid claims
	/// in its own `hit_test`, since a `Scroller` only takes a press aimed at its
	/// owner (U5.5). It only exists once the roster overflows.
	#[test]
	fn the_grid_claims_its_own_scrollbar_column() {
		let short = Rect::new(0.0, 0.0, 260.0, 200.0);
		let (_chrome, mut ui, id) = hosted(short, loaded_snap(0, false, None, 40, None));
		assert!(grid_of(&ui, id).scroller.has_bar(), "a short panel overflows");
		let bar = Vec2::new(short.right() - 4.0, short.bottom() - 4.0);
		assert!(ui.dispatch(&[press(true, bar)]).wants_pointer(), "the bar takes the press");

		let tall = Rect::new(0.0, 0.0, 260.0, 3000.0);
		let (_chrome, ui, id) = hosted(tall, loaded_snap(0, false, None, 40, None));
		assert!(!grid_of(&ui, id).scroller.has_bar(), "a tall panel fits the whole grid, so no bar");
	}

	/// `thumbnail_quads` composites body + turret into one cell (both anchored
	/// on the shared hotspot, tagged with the team, no shadow, inside the cell),
	/// draws body-only when there is no turret slot or the unit hides its
	/// turret, and nothing when the body slot is missing.
	#[test]
	fn thumbnail_quads_compose_body_and_turret_in_cell() {
		let data = BaseUnitData {
			image_base: 0,
			image_count: 8,
			turret_image_base: 8,
			turret_image_count: 8,
			..Default::default()
		};
		let u = unit("TANK", data, (0..16).map(|_| frame(32, 32, 16, 16)).collect(), Vec::new());
		let cell = Rect::new(100.0, 100.0, CELL, CELL);
		let body_meta = meta((0, 0), (32, 32));
		let turret_meta = meta((32, 0), (32, 32));

		let mut both = Vec::new();
		thumbnail_quads(&u, Some(&body_meta), Some(&turret_meta), 3, cell, &mut both);
		assert_eq!(both.len(), 2, "body + turret -> two quads");
		assert!(both.iter().all(|q| q.team == 3 && !q.shadow), "quads carry the team and never shadow in the picker");
		assert_eq!((both[0].origin, both[1].origin), ((0, 0), (32, 0)), "each quad samples its own atlas slot");
		for q in &both {
			assert!(q.rect.x >= cell.x - 0.5 && q.rect.y >= cell.y - 0.5, "the thumbnail stays inside its cell");
			assert!(q.rect.x + q.rect.w <= cell.x + CELL + 0.5 && q.rect.y + q.rect.h <= cell.y + CELL + 0.5);
		}

		let mut body_only = Vec::new();
		thumbnail_quads(&u, Some(&body_meta), None, 0, cell, &mut body_only);
		assert_eq!(body_only.len(), 1, "no turret slot -> body only");

		let scout = unit(
			"SCOUT",
			BaseUnitData { image_count: 16, ..Default::default() },
			(0..16).map(|_| frame(32, 32, 16, 16)).collect(),
			Vec::new(),
		);
		let mut hidden = Vec::new();
		thumbnail_quads(&scout, Some(&body_meta), Some(&turret_meta), 0, cell, &mut hidden);
		assert_eq!(hidden.len(), 1, "turret_image_count 0 -> the turret slot is ignored");

		let mut nobody = Vec::new();
		thumbnail_quads(&u, None, Some(&turret_meta), 0, cell, &mut nobody);
		assert!(nobody.is_empty(), "no body atlas slot -> no quads");
	}

	/// `quads` yields nothing until both the library and the atlas slots are
	/// present (the populated path needs a GPU-built `AtlasSlots`), and it lays
	/// its thumbnails on the cells the grid widget reported — one per sprite,
	/// each inside its own cell.
	#[test]
	fn quads_needs_both_library_and_slots() {
		let cells = [(0usize, Rect::new(10.0, 10.0, CELL, CELL)), (1, Rect::new(70.0, 10.0, CELL, CELL))];
		let lib = lib_of(&["ALPHA", "BETA"]);
		assert!(quads(None, None, 0, &cells).is_empty(), "no library -> no quads");
		assert!(quads(Some(&lib), None, 0, &cells).is_empty(), "no atlas slots -> no quads");
	}

	/// The panel draws on the base pass only — it hosts no popup layer, so the
	/// overlay pass carries nothing.
	#[test]
	fn the_panel_draws_on_the_base_pass_only() {
		let body = Rect::new(0.0, 0.0, 260.0, 400.0);
		let (chrome, ui, _id) = hosted(body, loaded_snap(0, false, Some(0), 12, Some("TANK")));
		assert!(!drawn(&chrome, &ui).cmds.is_empty(), "the base pass draws the units panel");

		let mut overlay = DrawList::new();
		ui.draw_pass(&mut overlay, chrome.theme(), chrome.fonts(), DrawPass::Overlay);
		assert!(overlay.cmds.is_empty(), "no overlay-pass drawing");
	}

	/// `UnitLibrary::load` reads the user's own MAX.RES roster: a sorted,
	/// non-empty unit list found case-insensitively. Retail data only - skips
	/// (loudly) when no MAX.RES is reachable (set `MAX_DIR` to cover it).
	#[test]
	fn library_load_roster_when_max_res_present() {
		let Some(dir) = std::env::var("MAX_DIR").ok().map(std::path::PathBuf::from).filter(|d| d.is_dir()) else {
			eprintln!("SKIPPED: units load - set MAX_DIR to a M.A.X. install to cover UnitLibrary::load");
			return;
		};
		if find_max_res(&dir).is_none() {
			eprintln!("SKIPPED: units load - no MAX.RES under {}", dir.display());
			return;
		}
		let lib = UnitLibrary::load(&dir).expect("a roster loads from MAX.RES");
		assert!(!lib.units.is_empty(), "the roster is non-empty");
		assert!(lib.units.windows(2).all(|w| w[0].tag <= w[1].tag), "the roster is sorted by tag");
		let first = lib.units[0].tag.clone();
		assert_eq!(lib.find(&first.to_ascii_lowercase()), Some(0), "find is case-insensitive over the roster");
	}

	/// `UnitLibrary::load` resolves connector-strut geometry for connector hosts,
	/// even the buildings/turrets that share a *class* template (D_LRGBLD /
	/// D_SMLBLD / D_FIXED) rather than a per-unit `D_<name>` - the frames the map
	/// overlay draws struts from. Retail data only (gated on `MAX_DIR`).
	#[test]
	fn connector_geometry_resolves_for_hosts() {
		let Some(dir) = std::env::var("MAX_DIR").ok().map(std::path::PathBuf::from).filter(|d| d.is_dir()) else {
			eprintln!("SKIPPED: connector geometry - set MAX_DIR to a M.A.X. install");
			return;
		};
		if find_max_res(&dir).is_none() {
			return;
		}
		let lib = UnitLibrary::load(&dir).expect("roster");
		// (tag, connector_image_base, connector_count) - matches the retained
		// per-unit bases in stock saves (SAVE-FORMAT / decode.rs).
		for (tag, base, count) in [
			("POWERSTN", 2, 8),  // D_LRGBLD (2×2 building)
			("COMMTWR", 2, 8),   // D_LRGBLD
			("POWGEN", 2, 4),    // D_SMLBLD (1×1 building)
			("CNCT_4W", 2, 4),   // D_SMLBLD (the connector unit itself)
			("GUNTURRT", 17, 4), // D_FIXED (fixed turret)
			("MININGST", 16, 8), // D_MINING (own template)
			("RADAR", 16, 4),    // D_RADAR (own template)
		] {
			let u = &lib.units[lib.find(tag).unwrap_or_else(|| panic!("{tag} in roster"))];
			assert_eq!(u.data.connector_image_base, base, "{tag} connector_image_base");
			assert_eq!(u.connector_count(), count, "{tag} connector_count");
			assert!(u.connector_frame(0).is_some(), "{tag} strut frame 0 atlased");
		}
		// A mobile unit and ground cover are never connector hosts.
		for tag in ["TANK", "LRGSLAB"] {
			let u = &lib.units[lib.find(tag).unwrap()];
			assert_eq!(u.connector_count(), 0, "{tag} draws no struts");
		}
	}
}
