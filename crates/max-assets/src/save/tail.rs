//! The save's **tail** (regions 23-25) — the part the re-emitter keeps as
//! bytes — taken apart far enough to follow the rest of the file: a change to
//! the **team types**, which decides its shape, and a change to the **object
//! graph**, which renumbers the references inside it.
//!
//! The tail is three regions, written by `saveload.cpp:715-733`:
//!
//! | region | content | shape |
//! | --- | --- | --- |
//! | 23 | heat maps | one `w*h*12`-byte block per **non-NONE** slot among 0-3 |
//! | 24 | message logs | four `u32` counts, each followed by that many entries |
//! | 25 | AI state | one `AiPlayer::FileSave` block per **COMPUTER** slot among 0-3 |
//!
//! So the tail's *shape* is a function of the team types, and re-typing a slot
//! without moving the tail with it desyncs every byte after the change — which
//! is why the Edit Save Data dialog used to lock the type of a NONE or COMPUTER
//! slot. [`retype`] does the move instead:
//!
//! - **region 23** is self-describing given `w`, `h` and the *old* types, so a
//!   slot joining the game gets an all-zero heat map spliced in at its place
//!   and a slot leaving loses its block. Every surviving slot keeps its own
//!   bytes.
//! - **region 25** needs the same treatment for the COMPUTER set, but its
//!   blocks are variable-length and sit *behind* region 24, so reaching them
//!   means parsing the message logs too. A slot becoming COMPUTER gets the
//!   block `AiPlayer::Init` would leave behind ([`fresh_ai_block`]); a slot
//!   leaving loses its own.
//!
//! Parsing is **exact or nothing**: if the logs and AI blocks do not consume
//! the tail to the last byte, [`retype`] refuses rather than emit a save whose
//! tail is a guess. A change that leaves the COMPUTER set alone never parses
//! past region 23 at all.
//!
//! ## The other half: the tail references the object graph
//!
//! A message-log line names the unit it is about, and an `AiPlayer` names every
//! unit it has spotted — by **on-disk object index**. So the tail is not inert
//! bytes that can be copied past a graph edit: adding a unit, deleting one, or
//! installing a per-unit stat override all renumber that index space, and the
//! tail has to move with it or come to name entirely different units.
//! [`remap`] does that, and it runs twice, for different reasons:
//!
//! - on the edit — [`follow_shift`], from `SaveFile::insert_object` /
//!   `remove_object` — so the stored bytes stay numbered in the model’s own
//!   vector space, the space every typed reference lives in;
//! - at emit — [`follow_graph`], because the serializer assigns indices by
//!   **encounter order**, which parts company with vector order the moment a
//!   unit is appended to a list partway through the walk.
//!
//! Both go through the graph’s own [`Writer`], so an inline body is re-emitted
//! from the model rather than patched in place, and its nested references
//! recurse. A reference whose object is *gone* is nulled in a message log (the
//! game writes such entries itself) and dropped from an AI’s spotted list — a
//! `SpottedUnit` has no null state.
//!
//! Both formats are walked. `V70` and `V71` differ only in index and count
//! widths, the heat cell (3 bytes vs 12) and an extra `AiPlayer` field —
//! [`retype`] on top is `V71`-only in practice, since Edit Save Data is gated on
//! `SaveFile::settings_regions_lossless`, which refuses `V70`.

use std::ops::Range;

use super::decode::TailWalker;
use super::error::EditError;
use super::serialize::{GraphOrder, Writer};
use super::types::{SaveFile, SaveFormat, TEAM_COUNT};

/// `TEAM_TYPE_NONE` — the slot takes no part in the game.
pub const TEAM_TYPE_NONE: u32 = 0;
/// `TEAM_TYPE_COMPUTER` — the slot is run by an `AiPlayer`, so it carries a
/// region-25 block.
pub const TEAM_TYPE_COMPUTER: u32 = 2;

/// Bytes per heat-map cell. `V71` stores a `HeatMapCell` — `complete`,
/// `stealth_sea`, `stealth_land`, three `u32`s (`heatmap.hpp`); `V70` stores the
/// same three fields as `int8`s, and as three separate planes rather than
/// interleaved (`HeatMap::LoadV70`) — which does not matter here, since only the
/// block's *size* is ever needed.
fn heat_cell(format: SaveFormat) -> usize {
	match format {
		SaveFormat::V70 => 3,
		SaveFormat::V71 => 12,
	}
}

/// The reader only ever reads heat maps for slots 0-3 (`saveload.cpp:1295`,
/// `PLAYER_TEAM_MAX - 1`), and the same bound picks the AI blocks
/// (`ai.cpp:175`). The alien slot never contributes to the tail.
const TAIL_SLOTS: usize = 4;

/// What the tail is measured against: the map it covers, the format that sets
/// every index and count width, and how many objects the graph before it
/// materialized (an object reference inside the tail is a back-index into
/// those, or the inline body of one past their end).
#[derive(Clone, Copy, Debug)]
pub struct TailShape {
	pub w: u16,
	pub h: u16,
	pub format: SaveFormat,
	pub objects: usize,
}

impl TailShape {
	/// The map's cell count — one heat-map cell and one AI map-plane byte each.
	fn cells(&self) -> usize {
		self.w as usize * self.h as usize
	}

	/// One team's whole region-23 block.
	fn heat_map_bytes(&self) -> usize {
		self.cells() * heat_cell(self.format)
	}
}

/// The slots that contribute a region-23 heat map, in file order.
fn heat_slots(types: &[u32; TEAM_COUNT]) -> Vec<usize> {
	(0..TAIL_SLOTS).filter(|&s| types[s] != TEAM_TYPE_NONE).collect()
}

/// The slots that contribute a region-25 AI block, in file order.
fn ai_slots(types: &[u32; TEAM_COUNT]) -> Vec<usize> {
	(0..TAIL_SLOTS).filter(|&s| types[s] == TEAM_TYPE_COMPUTER).collect()
}

/// The region-25 block for a slot that has just become COMPUTER: exactly the
/// state `AiPlayer::Init` leaves behind (`aiplayer.cpp:3027`) — strategy
/// `AI_STRATEGY_RANDOM`, no target, no spotted units, neither derived map
/// allocated — which is also what `Ai_FileLoad` starts every team from before
/// it reads. An AI restored from these bytes is one that has not taken a turn
/// yet, so it re-derives everything on its first `BeginTurn`.
fn fresh_ai_block(slot: usize) -> Vec<u8> {
	let mut out = Vec::with_capacity(25);
	out.extend_from_slice(&(slot as u16).to_le_bytes()); // player_team
	out.push(0); // strategy = AI_STRATEGY_RANDOM
	out.extend_from_slice(&(-1i16).to_le_bytes()); // greenhouse_ratio
	out.extend_from_slice(&(-1i16).to_le_bytes()); // minefield_density
	out.extend_from_slice(&(-1i16).to_le_bytes()); // target_team
	out.extend_from_slice(&0u32.to_le_bytes()); // spotted_units count
	out.extend_from_slice(&0u32.to_le_bytes()); // info_map absent
	out.extend_from_slice(&0u32.to_le_bytes()); // mine_map absent
	out.extend_from_slice(&0i16.to_le_bytes()); // target_location.x
	out.extend_from_slice(&0i16.to_le_bytes()); // target_location.y
	out
}
/// A record that carries exactly one object reference — a message-log entry or
/// a spotted unit — split into the bytes before the reference, the slot it
/// resolves to (`None` = a null reference), and the bytes after. The reference
/// itself is not kept as bytes: it is re-emitted, never copied.
#[derive(Clone, Debug)]
struct Entry {
	head: Range<usize>,
	unit: Option<usize>,
	tail: Range<usize>,
}

/// One region-25 block (`AiPlayer::FileSave`).
#[derive(Clone, Debug)]
struct AiBlock {
	/// The team the block belongs to.
	slot: usize,
	/// The whole block — what [`retype`] copies or drops.
	span: Range<usize>,
	/// Everything up to (not including) the spotted-unit count.
	head: Range<usize>,
	spotted: Vec<Entry>,
	/// The two derived map planes and `target_location`, after the spotted list.
	rest: Range<usize>,
	/// Whether the block inlined an object body of its own — a spotted unit the
	/// graph no longer holds. Such a block cannot be *dropped*: every reference
	/// after it is numbered against the object it added.
	inlines: bool,
}

/// The tail, taken apart: where its heat maps end, its four message logs, its
/// AI blocks, and the objects it materialized along the way.
///
/// Every range is a byte range into the whole tail. Building one is **exact or
/// nothing** — if the walk does not land on the last byte, the tail is not the
/// shape it was taken for, and neither re-emitting nor re-shaping it off a
/// guess is safe.
struct Plan {
	/// The end of region 23 — its blocks hold no references, so they are copied
	/// through as one span.
	heat_end: usize,
	logs: Vec<Vec<Entry>>,
	ai: Vec<AiBlock>,
	/// The graph's placeholder slots followed by the objects the tail inlined —
	/// what a [`Writer`] needs to emit those bodies again.
	records: Vec<super::types::SaveObject>,
	metas: Vec<super::types::ObjMeta>,
}

impl Plan {
	fn of(tail: &[u8], shape: &TailShape, types: &[u32; TEAM_COUNT]) -> Result<Plan, EditError> {
		if types[TAIL_SLOTS] != TEAM_TYPE_NONE {
			return Err(EditError::Tail(
				"this save's alien slot takes part in the game - the reader stops at slot 3".into(),
			));
		}
		let heat_end = heat_slots(types).len() * shape.heat_map_bytes();
		if tail.len() < heat_end {
			return Err(EditError::Tail("tail is shorter than the heat maps its team types call for".into()));
		}
		let mut c = TailWalker::new(tail, shape.format, shape.objects);
		c.skip(heat_end).map_err(|e| EditError::Tail(format!("tail: heat maps: {e}")))?;

		// Region 24 — four message logs, one per non-alien team
		// (`message_manager.cpp:271`). Each is a count followed by that many
		// `MessageLogEntry::FileSave` records: a `u16` text length, the text, an
		// object reference to the entry's unit, a `Point`, the alert flag, and
		// the icon's `ResourceID`.
		let mut logs = Vec::with_capacity(TAIL_SLOTS);
		for _ in 0..TAIL_SLOTS {
			let count = c.count().map_err(|e| EditError::Tail(format!("tail: message logs: {e}")))?;
			// A text length, a reference, a point, the flag and the icon id.
			let mut entries = Vec::with_capacity(c.capacity_for(count as usize, 2 + 2 + 4 + 1 + 2));
			for _ in 0..count {
				let from = c.pos();
				let len = c.u16().map_err(|e| EditError::Tail(format!("tail: message logs: {e}")))? as usize;
				c.skip(len).map_err(|e| EditError::Tail(format!("tail: message logs: {e}")))?; // text, NUL included
				let (slot, span) = c.object_ref().map_err(|e| EditError::Tail(format!("tail: message logs: {e}")))?;
				let head = from..span.start;
				let after = c.pos();
				c.skip(4 + 1 + 2).map_err(|e| EditError::Tail(format!("tail: message logs: {e}")))?; // point, alert flag, id
				entries.push(Entry { head, unit: slot, tail: after..c.pos() });
			}
			logs.push(entries);
		}

		// Region 25 — one `AiPlayer::FileSave` block per COMPUTER slot, in slot
		// order (`ai.cpp:175`).
		let mut ai = Vec::new();
		for slot in ai_slots(types) {
			let before = c.objects_seen();
			ai.push(ai_block(&mut c, slot, shape)?);
			if let Some(last) = ai.last_mut() {
				last.inlines = c.objects_seen() != before;
			}
		}
		if !c.at_end() {
			return Err(EditError::Tail(format!("tail: {} bytes left over after the AI state", tail.len() - c.pos())));
		}
		let (records, metas) = c.into_parts();
		Ok(Plan { heat_end, logs, ai, records, metas })
	}
}

/// Walks one region-25 block (`AiPlayer::FileSave`, `aiplayer.cpp:4247`).
/// `slot` is the team the block must claim — the blocks are written in slot
/// order, so a mismatch means the parse has drifted.
fn ai_block(c: &mut TailWalker, slot: usize, shape: &TailShape) -> Result<AiBlock, EditError> {
	let start = c.pos();
	let team = c.u16().map_err(|e| EditError::Tail(e.to_string()))?;
	if team as usize != slot {
		return Err(EditError::Tail(format!("tail: AI block claims team {team}, expected {slot}")));
	}
	// strategy, greenhouse_ratio, minefield_density, target_team — plus the
	// `unused_field` V70 reads between strategy and the ratios.
	c.skip(match shape.format {
		SaveFormat::V70 => 9,
		SaveFormat::V71 => 7,
	})
	.map_err(|e| EditError::Tail(e.to_string()))?;
	let head = start..c.pos();

	let spotted_count = c.count().map_err(|e| EditError::Tail(e.to_string()))?;
	// A reference, then team + visible_to_team + last_position.
	let mut spotted = Vec::with_capacity(c.capacity_for(spotted_count as usize, 2 + 2 + 1 + 4));
	for _ in 0..spotted_count {
		let from = c.pos();
		let (slot, span) = c.object_ref().map_err(|e| EditError::Tail(e.to_string()))?;
		let after = c.pos();
		c.skip(2 + 1 + 4).map_err(|e| EditError::Tail(e.to_string()))?; // team, visible_to_team, last_position
		spotted.push(Entry { head: from..span.start, unit: slot, tail: after..c.pos() });
	}

	// `info_map` then `mine_map`: a count of 1 (present) or 0 (never allocated),
	// each a full `w * h` byte plane when present.
	let rest_from = c.pos();
	for what in ["info map", "mine map"] {
		match c.count().map_err(|e| EditError::Tail(e.to_string()))? {
			0 => {}
			1 => c.skip(shape.cells()).map_err(|e| EditError::Tail(e.to_string()))?,
			n => return Err(EditError::Tail(format!("tail: AI {what} count {n} is neither 0 nor 1"))),
		}
	}
	c.skip(4).map_err(|e| EditError::Tail(e.to_string()))?; // target_location
	Ok(AiBlock { slot, span: start..c.pos(), head, spotted, rest: rest_from..c.pos(), inlines: false })
}

/// Rewrites `tail` so its object references follow `map`, a **current slot ->
/// new on-disk index** function over the graph in front of it. `new_objects` is
/// where the tail's own inline bodies carry on from — the count of objects the
/// graph emitted.
///
/// Two things move on a graph edit and both are handled here: a reference to an
/// object that shifted is renumbered, and one to an object that is *gone*
/// (`map` answers `None`) is either nulled or dropped. A message-log entry keeps
/// its line and loses its unit — the game already writes such entries
/// (`MessageLogEntry(text, id)` leaves the unit null) and `Select` checks for
/// it. A spotted unit is *removed* instead: `SpottedUnit` has no null state and
/// `UpdatePositionIfVisible` would dereference it.
pub(crate) fn remap(
	tail: &[u8],
	shape: &TailShape,
	types: &[u32; TEAM_COUNT],
	map: &dyn Fn(usize) -> Option<usize>,
	new_objects: usize,
) -> Result<Vec<u8>, EditError> {
	let plan = Plan::of(tail, shape, types)?;
	// A reference into the graph follows `map`; one to a body the tail inlined
	// is left to the writer, which re-numbers it as it re-emits it.
	let gone = |r: Option<usize>| matches!(r, Some(i) if i < shape.objects && map(i).is_none());

	let mut emitted: Vec<Option<usize>> = vec![None; plan.records.len()];
	for (i, e) in emitted.iter_mut().enumerate().take(shape.objects) {
		*e = map(i);
	}
	let mut w = Writer::for_tail(&plan.records, &plan.metas, shape.format, emitted, new_objects);

	w.raw(&tail[..plan.heat_end]);
	for log in &plan.logs {
		w.count(log.len() as u32);
		for e in log {
			w.raw(&tail[e.head.clone()]);
			// A vanished unit leaves the line, not the reference.
			w.reference(if gone(e.unit) { None } else { e.unit });
			w.raw(&tail[e.tail.clone()]);
		}
	}
	for b in &plan.ai {
		w.raw(&tail[b.head.clone()]);
		let kept: Vec<&Entry> = b.spotted.iter().filter(|e| !gone(e.unit)).collect();
		w.count(kept.len() as u32);
		for e in kept {
			w.raw(&tail[e.head.clone()]);
			w.reference(e.unit);
			w.raw(&tail[e.tail.clone()]);
		}
		w.raw(&tail[b.rest.clone()]);
	}
	Ok(w.into_vec())
}

/// The tail for a save whose graph has just been re-emitted: its references are
/// numbered in that graph's index space, so they follow the walk's answer rather
/// than the model's vector order.
///
/// The overwhelmingly common case is that the two agree — nothing was added or
/// removed — and then the stored bytes are already right and are copied through
/// untouched, which is what keeps an unedited save byte-exact.
///
/// A tail that will not decompose cannot be moved, and the stored bytes are no
/// longer valid for a graph that has shifted under them: their references would
/// point at the wrong units. That is the corruption this whole module exists to
/// prevent, so it is an error, not a fallback - callers that can say so earlier
/// check [`SaveFile::tail_follows_the_graph`] *before* a graph-structural edit.
pub(crate) fn follow_graph(save: &SaveFile, order: &GraphOrder) -> Result<Vec<u8>, EditError> {
	if order.is_identity(save.objects.len()) {
		return Ok(save.raw.tail.clone());
	}
	let shape = save.tail_shape();
	let map = |i: usize| order.emitted.get(i).copied().flatten();
	remap(&save.raw.tail, &shape, &save.header.team_type, &map, order.next_emit)
		.map_err(|e| EditError::Tail(format!("the graph moved but its tail will not follow: {e}")))
}

/// How a graph edit moved the objects the tail references.
pub(crate) enum Shift {
	/// `insert_object(at)` — every slot from `at` up moved one along.
	Inserted(usize),
	/// `remove_object(at)` — slot `at` is gone and every slot above it moved
	/// one back.
	Removed(usize),
}

impl Shift {
	fn map(&self, i: usize) -> Option<usize> {
		match *self {
			Shift::Inserted(at) => Some(if i >= at { i + 1 } else { i }),
			Shift::Removed(at) => match i.cmp(&at) {
				std::cmp::Ordering::Less => Some(i),
				std::cmp::Ordering::Equal => None,
				std::cmp::Ordering::Greater => Some(i - 1),
			},
		}
	}
}

/// Move a tail with the graph it references, for a `shift` about to be applied
/// to `save.objects`. Call **before** the vector is mutated: the walk is
/// measured against the graph as it stands.
///
/// This is the other half of [`follow_graph`]. That one renumbers at emit time,
/// against the walk order; this one keeps the *stored* tail numbered against the
/// model's vector, which is the space every other reference in the model lives
/// in ([`SaveFile::remap_indices`]).
pub(crate) fn follow_shift(save: &SaveFile, shift: &Shift) -> Result<Vec<u8>, EditError> {
	let after = match *shift {
		Shift::Inserted(_) => save.objects.len() + 1,
		Shift::Removed(_) => save.objects.len() - 1,
	};
	remap(&save.raw.tail, &save.tail_shape(), &save.header.team_type, &|i| shift.map(i), after)
}

/// Rewrites `tail` so its shape matches `new` instead of `old`.
///
/// `shape` is what the tail is measured against — see [`TailShape`]. Returns
/// the new tail bytes, or `Err` describing why the tail could not be moved, in
/// which case the caller must **not** apply the type change.
///
/// A team dropped out of the game and put back loses its old heat map (the
/// slot re-enters with an all-zero one): the bytes are gone the moment they
/// leave the file, so an undo of such an edit restores the team, not its
/// explored-terrain record.
pub fn retype(
	tail: &[u8],
	shape: &TailShape,
	old: &[u32; TEAM_COUNT],
	new: &[u32; TEAM_COUNT],
) -> Result<Vec<u8>, EditError> {
	if old == new {
		return Ok(tail.to_vec());
	}
	// The writer emits a heat map for the alien slot too, but the reader stops
	// at slot 3 (`SAVE-FROM-SCRATCH.md` §6.3) - a save with a live alien slot
	// is one the game itself could not load, so neither end of that change is
	// something this can produce.
	if old[TAIL_SLOTS] != new[TAIL_SLOTS] {
		return Err(EditError::Tail("the alien slot's type is fixed - the game's own reader stops at slot 3".into()));
	}
	if old[TAIL_SLOTS] != TEAM_TYPE_NONE {
		return Err(EditError::Tail(
			"this save's alien slot takes part in the game - its tail cannot be re-shaped".into(),
		));
	}
	let map_bytes = shape.heat_map_bytes();
	let old_heat = heat_slots(old);
	let split = old_heat.len() * map_bytes;
	if tail.len() < split {
		return Err(EditError::Tail("tail is shorter than the heat maps its team types call for".into()));
	}

	let mut out = Vec::with_capacity(tail.len() + map_bytes);
	for slot in heat_slots(new) {
		match old_heat.iter().position(|&s| s == slot) {
			// A slot that was already in the game keeps its own heat map.
			Some(i) => out.extend_from_slice(&tail[i * map_bytes..(i + 1) * map_bytes]),
			// One joining starts with nothing seen - the same all-zero map
			// `ResourceManager_InitHeatMaps` would hand it.
			None => out.resize(out.len() + map_bytes, 0),
		}
	}

	let old_ai = ai_slots(old);
	let new_ai = ai_slots(new);
	if old_ai == new_ai {
		// Regions 24-25 are untouched by a change that leaves the AI set alone
		// - no need to parse a single byte of them.
		out.extend_from_slice(&tail[split..]);
		return Ok(out);
	}

	let plan = Plan::of(tail, shape, old)?;
	if let Some(b) = plan.ai.iter().find(|b| b.inlines && !new_ai.contains(&b.slot)) {
		return Err(EditError::Tail(format!(
			"team {}'s AI state carries a unit body the graph no longer holds - dropping it would strand every \
			 object reference after it",
			b.slot
		)));
	}
	let logs_end = plan.ai.first().map(|b| b.span.start).unwrap_or(tail.len());
	out.extend_from_slice(&tail[plan.heat_end..logs_end]);
	for slot in new_ai {
		match plan.ai.iter().find(|b| b.slot == slot) {
			Some(b) => out.extend_from_slice(&tail[b.span.clone()]),
			None => out.extend_from_slice(&fresh_ai_block(slot)),
		}
	}
	Ok(out)
}

/// Every unit the tail's message logs and AI state name, in stream order —
/// `None` where the reference is null or does not resolve to a unit.
///
/// A diagnostic, and the way to check the thing that is easy to get wrong: a
/// graph edit renumbers object references, and these are the ones that live
/// outside the typed model. Run it before and after an edit and the two lists
/// must match — a tail that stopped following would name *different* units.
pub fn referenced_units(save: &SaveFile) -> Result<Vec<Option<u16>>, EditError> {
	let plan = Plan::of(&save.raw.tail, &save.tail_shape(), &save.header.team_type)?;
	let graph = save.objects.len();
	let id_of = |slot: Option<usize>| -> Option<u16> {
		// A graph slot reads off the model; one past it is a body the tail
		// inlined, which only the walk materialized.
		let rec = match slot? {
			i if i < graph => save.objects.get(i),
			i => plan.records.get(i),
		};
		match rec {
			Some(super::types::SaveObject::Unit(u)) => Some(u.id),
			_ => None,
		}
	};
	let logs = plan.logs.iter().flatten();
	let spotted = plan.ai.iter().flat_map(|b| b.spotted.iter());
	Ok(logs.chain(spotted).map(|e| id_of(e.unit)).collect())
}

/// Whether this tail decomposes exactly — i.e. whether it can be moved for
/// **any** change: a team type that adds or removes an AI block ([`retype`]),
/// or a graph edit that renumbers its object references ([`remap`]).
///
/// `false` narrows the offer to team-type changes that leave the COMPUTER set
/// alone (those never read past region 23) and rules out adding or removing an
/// object at all.
pub fn decomposes(tail: &[u8], shape: &TailShape, types: &[u32; TEAM_COUNT]) -> bool {
	Plan::of(tail, shape, types).is_ok()
}

#[cfg(test)]
mod tests {
	use super::*;

	const W: u16 = 4;
	const H: u16 = 3;
	const MAP: usize = (W as usize) * (H as usize) * 12; // V71 heat cells

	/// The fixtures below build their own tails, so the object graph is empty
	/// (no tail reference points into one) - `objects` only matters for a real
	/// save's message logs, which [`message_log_entries_are_walked_not_guessed`]
	/// covers explicitly.
	const SHAPE: TailShape = TailShape { w: W, h: H, format: SaveFormat::V71, objects: 0 };

	/// A tail whose message log and AI state both point into the graph: one log
	/// line naming object 7, and one AI block whose single spotted unit names
	/// object 3. Built for `objects` graph slots, `types` = Red player + Green
	/// computer.
	fn tail_with_refs(objects: usize) -> (Vec<u8>, TailShape, [u32; TEAM_COUNT]) {
		let types = [1, 2, 0, 0, 0];
		let shape = TailShape { objects, ..SHAPE };
		let mut t = vec![0u8; 2 * MAP];
		// Red's log: one entry naming object 7 (disk index 8).
		t.extend_from_slice(&1u32.to_le_bytes());
		t.extend_from_slice(&3u16.to_le_bytes());
		t.extend_from_slice(b"hi\0");
		t.extend_from_slice(&8u32.to_le_bytes()); // unit = slot 7
		t.extend_from_slice(&[0; 4]); // point
		t.push(1); // is_alert_message
		t.extend_from_slice(&0xFFFFu16.to_le_bytes()); // id
		for _ in 1..TAIL_SLOTS {
			t.extend_from_slice(&0u32.to_le_bytes());
		}
		// Green's AI block with one spotted unit naming object 3 (disk index 4).
		t.extend_from_slice(&1u16.to_le_bytes()); // player_team
		t.extend_from_slice(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]); // strategy + three i16
		t.extend_from_slice(&1u32.to_le_bytes()); // one spotted unit
		t.extend_from_slice(&4u32.to_le_bytes()); // unit = slot 3
		t.extend_from_slice(&[0; 2 + 1 + 4]); // team, visible, last_position
		t.extend_from_slice(&0u32.to_le_bytes()); // no info map
		t.extend_from_slice(&0u32.to_le_bytes()); // no mine map
		t.extend_from_slice(&[0; 4]); // target_location
		(t, shape, types)
	}

	/// Every disk index the tail holds, in stream order — the log's unit then
	/// the AI block's spotted unit.
	fn refs_of(tail: &[u8]) -> Vec<u32> {
		let at = |o: usize| u32::from_le_bytes(tail[o..o + 4].try_into().unwrap());
		let log_ref = 2 * MAP + 4 + 2 + 3;
		let ai_ref = log_ref + 4 + 4 + 1 + 2 + 3 * 4 + 2 + 7 + 4;
		vec![at(log_ref), at(ai_ref)]
	}

	/// The headline of the fix: an object inserted into the graph shifts every
	/// slot above it, and the tail's references shift with it. Without this the
	/// message log and the AI would silently come to name *different* units.
	#[test]
	fn an_inserted_object_moves_the_tails_references() {
		let (before, shape, types) = tail_with_refs(10);
		assert_eq!(refs_of(&before), vec![8, 4], "slots 7 and 3");

		// Insert at slot 5: slot 7 moves to 8, slot 3 stays put.
		let after = remap(&before, &shape, &types, &|i| Some(if i >= 5 { i + 1 } else { i }), 11).expect("it moves");
		assert_eq!(refs_of(&after), vec![9, 4], "the one above the insert moved, the one below did not");
		assert_eq!(after.len(), before.len(), "and nothing else changed size");
	}

	/// A reference that survives untouched re-emits byte-identically — the
	/// property that keeps an unedited save byte-exact.
	#[test]
	fn an_identity_remap_is_a_no_op() {
		let (before, shape, types) = tail_with_refs(10);
		let after = remap(&before, &shape, &types, &|i| Some(i), 10).expect("it moves");
		assert_eq!(after, before, "byte-for-byte");
	}

	/// The object a reference names can also be *gone*. A message-log line keeps
	/// its text and loses its unit (the game writes such entries itself); a
	/// spotted unit is dropped outright, because `SpottedUnit` has no null state.
	#[test]
	fn a_removed_object_nulls_a_log_line_and_drops_a_spotted_unit() {
		let (before, shape, types) = tail_with_refs(10);

		// Slot 7 is gone: the log line stays, its unit reference goes null.
		let after = remap(&before, &shape, &types, &|i| (i != 7).then_some(i), 10).expect("it moves");
		assert_eq!(refs_of(&after)[0], 0, "the line lost its unit");
		assert_eq!(after.len(), before.len(), "but not its text");

		// Slot 3 is gone: the AI's only spotted unit goes with it.
		let after = remap(&before, &shape, &types, &|i| (i != 3).then_some(i), 10).expect("it moves");
		let count_at = 2 * MAP + 4 + 2 + 3 + 4 + 4 + 1 + 2 + 3 * 4 + 2 + 7;
		assert_eq!(u32::from_le_bytes(after[count_at..count_at + 4].try_into().unwrap()), 0, "no spotted units left");
		assert_eq!(after.len(), before.len() - (4 + 2 + 1 + 4), "one spotted-unit record shorter");
	}

	/// Nothing in the tail is copied through blind: a run over the real fixture
	/// proves the whole shape - inline bodies included - re-emits byte-exactly
	/// when nothing has moved, and follows a shift when something has.
	#[test]
	fn a_real_saves_tail_re_emits_and_follows_when_present() {
		use super::super::encode::tests::load_fixture;
		let Some((_raw, save)) = load_fixture() else { return };
		let shape = save.tail_shape();
		let types = save.header.team_type;

		let same = remap(&save.raw.tail, &shape, &types, &|i| Some(i), shape.objects).expect("it walks");
		assert_eq!(same, save.raw.tail, "an identity remap of a real tail is byte-exact");

		// Insert at the front: every reference the tail holds moves up one, and
		// so does the index of the unit body it inlines.
		let moved = remap(&save.raw.tail, &shape, &types, &|i| Some(i + 1), shape.objects + 1).expect("it moves");
		assert_ne!(moved, save.raw.tail, "the references moved");
		assert_eq!(moved.len(), save.raw.tail.len(), "without changing size");
		// And the result is still a tail: it walks against the grown graph.
		let grown = TailShape { objects: shape.objects + 1, ..shape };
		assert!(decomposes(&moved, &grown, &types), "the moved tail decomposes against the graph it now names");
	}

	fn types(t: [u32; TEAM_COUNT]) -> [u32; TEAM_COUNT] {
		t
	}

	/// A tail for `types`: each active slot's heat map filled with its own slot
	/// number (so a splice that moved the wrong block is visible), four empty
	/// message logs, then a fresh AI block per COMPUTER slot.
	fn tail_for(t: &[u32; TEAM_COUNT]) -> Vec<u8> {
		let mut out = Vec::new();
		for slot in heat_slots(t) {
			out.resize(out.len() + MAP, slot as u8 + 1);
		}
		for _ in 0..TAIL_SLOTS {
			out.extend_from_slice(&0u32.to_le_bytes());
		}
		for slot in ai_slots(t) {
			out.extend_from_slice(&fresh_ai_block(slot));
		}
		out
	}

	/// The `n`th heat-map block of a tail built for `t`.
	fn map_of(tail: &[u8], t: &[u32; TEAM_COUNT], slot: usize) -> Vec<u8> {
		let i = heat_slots(t).iter().position(|&s| s == slot).expect("an active slot");
		tail[i * MAP..(i + 1) * MAP].to_vec()
	}

	/// The headline case: a slot that took no part gets a type, and the tail
	/// grows one all-zero heat map **in slot order** - every other team's map
	/// stays byte-identical and where the reader expects it.
	#[test]
	fn a_none_slot_joining_splices_a_zero_heat_map_in_place() {
		let old = types([1, 0, 2, 0, 0]);
		let new = types([1, 3, 2, 0, 0]);
		let before = tail_for(&old);

		let after = retype(&before, &SHAPE, &old, &new).expect("the tail moves");

		assert_eq!(after.len(), before.len() + MAP, "one heat map wider");
		assert_eq!(map_of(&after, &new, 0), map_of(&before, &old, 0), "Red keeps its own map");
		assert_eq!(map_of(&after, &new, 1), vec![0u8; MAP], "Green joins with nothing seen");
		assert_eq!(map_of(&after, &new, 2), map_of(&before, &old, 2), "Blue keeps its own map");
		assert_eq!(&after[3 * MAP..], &before[2 * MAP..], "and regions 24-25 are untouched");
	}

	/// The reverse: a slot leaving the game drops its block, and the ones after
	/// it close up.
	#[test]
	fn a_slot_leaving_drops_its_heat_map() {
		let old = types([1, 4, 1, 0, 0]);
		let new = types([1, 0, 1, 0, 0]);
		let before = tail_for(&old);

		let after = retype(&before, &SHAPE, &old, &new).expect("the tail moves");

		assert_eq!(after.len(), before.len() - MAP, "one heat map narrower");
		assert_eq!(map_of(&after, &new, 0), map_of(&before, &old, 0), "Red is untouched");
		assert_eq!(map_of(&after, &new, 2), map_of(&before, &old, 2), "and Blue moved up intact");
	}

	/// Turning a team over to the AI appends the block `AiPlayer::Init` leaves
	/// behind, in slot order among the AI blocks already there.
	#[test]
	fn becoming_computer_synthesizes_an_ai_block() {
		let old = types([1, 1, 2, 0, 0]);
		let new = types([1, 2, 2, 0, 0]);
		let before = tail_for(&old);

		let after = retype(&before, &SHAPE, &old, &new).expect("the tail moves");

		assert_eq!(after.len(), before.len() + fresh_ai_block(1).len(), "one AI block longer");
		// Three active slots, then the four empty message logs.
		let logs = 3 * MAP + 4 * 4;
		assert_eq!(&after[logs..logs + 25], &fresh_ai_block(1)[..], "Green's block comes first");
		assert_eq!(&after[logs + 25..], &fresh_ai_block(2)[..], "then Blue's, still in slot order");
	}

	/// And the reverse - a team taken off the AI loses its block, the rest kept
	/// byte-for-byte.
	#[test]
	fn leaving_computer_drops_its_ai_block() {
		let old = types([2, 1, 2, 0, 0]);
		let new = types([1, 1, 2, 0, 0]);
		let before = tail_for(&old);

		let after = retype(&before, &SHAPE, &old, &new).expect("the tail moves");

		let logs = 3 * MAP + 4 * 4;
		assert_eq!(after.len(), before.len() - 25);
		assert_eq!(&after[logs..], &fresh_ai_block(2)[..], "only Blue's block is left");
	}

	/// A tail whose variable half does not decompose is refused - but only when
	/// the change actually needs to reach it. A swap inside {Player, Remote,
	/// Eliminated} still goes through, because it never parses past region 23.
	#[test]
	fn a_tail_that_does_not_decompose_is_refused_only_when_it_must_be_read() {
		let old = types([1, 2, 0, 0, 0]);
		let mut junk = tail_for(&old);
		junk.truncate(junk.len() - 3); // cut into the AI block

		assert!(!decomposes(&junk, &SHAPE, &old), "the tail no longer decomposes");
		assert!(retype(&junk, &SHAPE, &old, &types([1, 1, 0, 0, 0])).is_err(), "an AI-set change is refused");
		let swap = retype(&junk, &SHAPE, &old, &types([3, 2, 0, 0, 0])).expect("a Player->Remote swap still works");
		assert_eq!(swap, junk, "byte-identical - it never reached the AI blocks");
	}

	/// A message log with entries in it still parses, so an AI-set change on a
	/// played-in save works. The entry shape is the one `MessageLogEntry` writes.
	#[test]
	fn message_log_entries_are_walked_not_guessed() {
		let old = types([1, 2, 0, 0, 0]);
		let mut tail = vec![0u8; 2 * MAP];
		// Red's log: one entry naming object 7; the other three are empty.
		tail.extend_from_slice(&1u32.to_le_bytes());
		tail.extend_from_slice(&5u16.to_le_bytes());
		tail.extend_from_slice(b"hi\0\0\0");
		tail.extend_from_slice(&7u32.to_le_bytes()); // unit
		tail.extend_from_slice(&[0; 4]); // point
		tail.push(1); // is_alert_message
		tail.extend_from_slice(&0u16.to_le_bytes()); // id
		for _ in 1..TAIL_SLOTS {
			tail.extend_from_slice(&0u32.to_le_bytes());
		}
		let logs_len = tail.len() - 2 * MAP;
		tail.extend_from_slice(&fresh_ai_block(1));

		assert!(
			decomposes(&tail, &TailShape { objects: 9, ..SHAPE }, &old),
			"an object index inside the graph is fine"
		);
		assert!(
			!decomposes(&tail, &TailShape { objects: 6, ..SHAPE }, &old),
			"one past its end would be an inline object"
		);

		let after =
			retype(&tail, &TailShape { objects: 9, ..SHAPE }, &old, &types([1, 1, 0, 0, 0])).expect("the tail moves");
		assert_eq!(after.len(), 2 * MAP + logs_len, "the AI block went, the logs stayed");
		assert_eq!(&after[2 * MAP..], &tail[2 * MAP..2 * MAP + logs_len], "byte-for-byte");
	}

	/// An AI block carrying both derived map planes is measured, not assumed -
	/// the planes are `w * h` bytes each and sit between the counts.
	#[test]
	fn an_ai_block_with_both_map_planes_is_measured() {
		let old = types([2, 0, 0, 0, 0]);
		let cells = W as usize * H as usize;
		let mut block = Vec::new();
		block.extend_from_slice(&0u16.to_le_bytes()); // player_team
		block.extend_from_slice(&[7, 0, 0, 0, 0, 0, 0]); // strategy + three i16s
		block.extend_from_slice(&0u32.to_le_bytes()); // no spotted units
		block.extend_from_slice(&1u32.to_le_bytes());
		block.resize(block.len() + cells, 0xAB); // info_map
		block.extend_from_slice(&1u32.to_le_bytes());
		block.resize(block.len() + cells, 0xCD); // mine_map
		block.extend_from_slice(&[0; 4]); // target_location

		let mut tail = vec![0u8; MAP];
		tail.extend_from_slice(&[0; 16]); // four empty logs
		tail.extend_from_slice(&block);

		assert!(decomposes(&tail, &SHAPE, &old), "both planes are accounted for");
		let after = retype(&tail, &SHAPE, &old, &types([1, 0, 0, 0, 0])).expect("the tail moves");
		assert_eq!(after.len(), tail.len() - block.len(), "and the whole block came out");
	}

	/// The synthesized block is exactly what the parser expects to read back -
	/// the round-trip that keeps [`fresh_ai_block`] and [`ai_block`] honest.
	#[test]
	fn a_synthesized_ai_block_parses_as_one() {
		let block = fresh_ai_block(2);
		let mut c = TailWalker::new(&block, SaveFormat::V71, 0);
		let parsed = ai_block(&mut c, 2, &SHAPE).expect("it parses");
		assert_eq!(parsed.span, 0..block.len(), "and consumes exactly itself");
		assert!(c.at_end(), "to the last byte");
	}
}
