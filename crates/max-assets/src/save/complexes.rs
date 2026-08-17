//! The per-team `Complex` invariant: analysis ([`check_complexes`]) and repair
//! ([`repair_complexes`]) — the HANDOFF 2026-08-02 Finding 1 fix.
//!
//! The engine attaches a `Complex` to every **connector host** at construction
//! (`unitinfo.cpp:310`) and `UnitInfo::FileLoad` reads the stored reference
//! verbatim — there is **no load-time fixup** — so a host saved with a null
//! complex is dereferenced unguarded at run time
//! (`Access_UpdateResourcesTotal`'s first statement is `complex->material = 0`).
//! That is the defect this module exists to close: the editor's placed
//! buildings used to export with `complex: None`.
//!
//! **What loading actually requires is weaker than the engine's steady state.**
//! Shipped stock data proves the engine tolerates drift the DOS-era tools left
//! behind, so the checker/repair here draw the line at what is dangerous or
//! editor-created, and leave tolerated drift byte-identical:
//!
//! | state | verdict | evidence |
//! | --- | --- | --- |
//! | host with a null / wrong-team complex | **repair** (crash class) | `unitinfo.cpp:310` vs `access.cpp:913` |
//! | `buildings` below the real membership | **repair** (`Shrink` hits 0 early and collects a live complex) | `complex.cpp:207` |
//! | team list unsorted / ids duplicated or < 1 | **repair** (`CreateComplex`'s gap scan assumes sorted-unique) | `teamunits.cpp:138` |
//! | one complex serving two *disconnected* fragments | **repair** (the state `DetachComplex` exists to undo; editor deletions create it) | `unitinfo.cpp:2255` |
//! | `buildings` running high | tolerated | SAVE3.SCE ships id 4 counting 14 with 13 members |
//! | a listed complex with no members | tolerated when the pristine save carried it (`keep_dead`); collected when an edit emptied it | SAVE16.SCE ships one; `Shrink` collection simply never runs |
//! | two complexes inside one connected cluster | tolerated | SAVE8.CAM ships a 67-building cluster split 2 ways |
//! | a host mid-construction with a null complex | tolerated | SAVE9.SCE ships two; the engine attaches at completion |
//!
//! The repair therefore never renumbers or normalizes globally. It mirrors the
//! engine's own **event** handlers, applied only where an edit left a hole:
//! an orphaned host runs `AttachToPrimaryComplex` (lowest-id connected
//! neighbour's complex wins, else `CreateComplex`) followed by the
//! `AttachComplex` flood along the unit's own connector bits — which is also
//! what merges bridged complexes into the lowest id; a complex left spanning
//! disconnected fragments runs the `DetachComplex` re-walk (each later
//! fragment gets a fresh complex). A save the engine wrote — or tolerated
//! drift included — sees **zero changes**, keeping the export identity.
//!
//! Only the four player teams serialize a `TeamUnits` table; an alien/derelict
//! host's complex lives in no list and is first-seen at the unit's own
//! `complex` reference instead.

use super::error::EditError;
use super::orders::{ORDER_BUILD, ORDER_IDLE, ORDER_STATE_BUILD_CLEARING, ORDER_STATE_SELECT_SITE};
use super::serialize::serialize_complex;
use super::types::{Complex, ObjMeta, SaveFile, SaveObject, UnitRecord};
use super::unit_types::flag::{BUILDING, CONNECTOR_UNIT, GROUND_COVER, STANDALONE};
use std::collections::{HashMap, HashSet};

/// Registry type index of `Complex` (`smartfile` class order).
const COMPLEX_TYPE: u32 = 3;

/// The eight connector half-edge bits (`enums.hpp`), in [`connector_neighbor`]'s
/// query order: NL, NR, ET, EB, SL, SR, WT, WB.
pub const CONNECTOR_BITS: [u16; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// The eight half-edge bits together. Higher stored bits (the engine's transient
/// `CONNECTION_BEING_TESTED`, 0x400) are not adjacency.
const CONNECTOR_MASK: u16 = 0xFF;

/// The neighbour cell a connector half-edge `bit` points at, for a host whose
/// top-left cell is `(x, y)` and whose `unit_size` is `s` (2 for a building,
/// else 1) — transcribed from `UnitsManager_RemoveConnections`
/// (`units_manager.cpp`), and validated against every stock save's stored masks.
/// NL/ET/SL/WT are the four sides a 1×1 uses; a 2×2 adds NR/EB/SR/WB one cell
/// further along each side.
pub fn connector_neighbor(x: i32, y: i32, s: i32, bit: u16) -> (i32, i32) {
	match bit {
		0x01 => (x, y - 1),     // NL
		0x02 => (x + 1, y - 1), // NR
		0x04 => (x + s, y),     // ET
		0x08 => (x + s, y + 1), // EB
		0x10 => (x, y + s),     // SL
		0x20 => (x + 1, y + s), // SR
		0x40 => (x - 1, y),     // WT
		_ => (x - 1, y + 1),    // WB (0x80)
	}
}

/// Whether this unit gets a `Complex` attached at construction — the engine's
/// own flag rule (`unitinfo.cpp:310`): `(CONNECTOR_UNIT | BUILDING | STANDALONE)
/// && !GROUND_COVER`, and a real id (`0xFFFF` marks a unit the deploy path
/// created as "existing", which the constructor skips).
fn is_complex_host(u: &UnitRecord) -> bool {
	u.flags & (CONNECTOR_UNIT | BUILDING | STANDALONE) != 0 && u.flags & GROUND_COVER == 0 && u.id != 0xFFFF
}

/// The engine's `unit_size`: 2 cells a side for a `BUILDING`, else 1.
fn unit_size(u: &UnitRecord) -> i32 {
	if u.flags & BUILDING != 0 { 2 } else { 1 }
}

/// Footprint occupancy over the complex hosts: every covered cell -> the host's
/// object slot. The stand-in for the engine's `Hash_MapHash` lookup inside
/// `Access_GetTeamBuilding`.
fn occupancy(save: &SaveFile) -> HashMap<(i32, i32), usize> {
	let mut occ = HashMap::new();
	for (slot, o) in save.objects.iter().enumerate() {
		let SaveObject::Unit(u) = o else { continue };
		if !is_complex_host(u) {
			continue;
		}
		let s = unit_size(u);
		for dx in 0..s {
			for dy in 0..s {
				occ.insert((u.grid_x as i32 + dx, u.grid_y as i32 + dy), slot);
			}
		}
	}
	occ
}

/// The hosts unit `slot`'s own set connector bits point at — the engine's
/// `GetConnectedBuilding` per bit: a same-team host covering the pointed-at
/// cell, not stored/idle (`Access_GetTeamBuilding` skips `ORDER_IDLE`).
fn connected_neighbors(save: &SaveFile, occ: &HashMap<(i32, i32), usize>, slot: usize) -> Vec<usize> {
	let Some(u) = save.unit(slot) else { return Vec::new() };
	let s = unit_size(u);
	let mask = u.connectors & CONNECTOR_MASK;
	let mut out = Vec::new();
	for bit in CONNECTOR_BITS {
		if mask & bit == 0 {
			continue;
		}
		let cell = connector_neighbor(u.grid_x as i32, u.grid_y as i32, s, bit);
		let Some(&t) = occ.get(&cell) else { continue };
		let v = save.unit(t).expect("occupancy maps to units");
		if t != slot && v.team == u.team && v.orders != ORDER_IDLE && !out.contains(&t) {
			out.push(t);
		}
	}
	out
}

/// The `AttachComplex` flood's reach from `seed`: the unit slots found by
/// recursively following each reached unit's **own** connector bits
/// (`unitinfo.cpp:1958` — the engine walks the unit's own half-edges only, so
/// an inbound-only connection does not propagate). Includes `seed`.
fn attach_reach(save: &SaveFile, occ: &HashMap<(i32, i32), usize>, seed: usize) -> Vec<usize> {
	let mut reached = vec![seed];
	let mut queue = vec![seed];
	while let Some(s) = queue.pop() {
		for t in connected_neighbors(save, occ, s) {
			if !reached.contains(&t) {
				reached.push(t);
				queue.push(t);
			}
		}
	}
	reached.sort_unstable();
	reached
}

/// One same-team connected cluster of complex hosts (the undirected closure of
/// the half-edge graph), members in ascending slot order. Used to detect a
/// complex left spanning *disconnected* fragments; the engine's own floods walk
/// directed reach ([`attach_reach`]) instead.
struct Component {
	team: u8,
	members: Vec<usize>,
}

/// Union-find root with path halving.
fn root(parent: &mut [usize], mut i: usize) -> usize {
	while parent[i] != i {
		parent[i] = parent[parent[i]];
		i = parent[i];
	}
	i
}

/// The connected components of `save`'s complex hosts under the connector
/// half-edge graph (edges from either endpoint's own bits), ordered by
/// smallest member slot.
fn components(save: &SaveFile) -> Vec<Component> {
	let occ = occupancy(save);
	let hosts: Vec<usize> = (0..save.objects.len()).filter(|&s| save.unit(s).is_some_and(is_complex_host)).collect();
	let index_of: HashMap<usize, usize> = hosts.iter().enumerate().map(|(hi, &s)| (s, hi)).collect();

	let mut parent: Vec<usize> = (0..hosts.len()).collect();
	for (hi, &slot) in hosts.iter().enumerate() {
		for t in connected_neighbors(save, &occ, slot) {
			let ti = index_of[&t];
			let (ra, rb) = (root(&mut parent, hi), root(&mut parent, ti));
			parent[ra] = rb;
		}
	}

	let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
	for (hi, &slot) in hosts.iter().enumerate() {
		let r = root(&mut parent, hi);
		by_root.entry(r).or_default().push(slot);
	}
	let mut comps: Vec<Component> = by_root
		.into_values()
		.map(|mut members| {
			members.sort_unstable();
			let team = save.unit(members[0]).expect("host slot holds a unit").team;
			Component { team, members }
		})
		.collect();
	comps.sort_by_key(|c| c.members[0]);
	comps
}

/// The id of the `Complex` at object slot `c`, if that slot holds one.
fn complex_id(save: &SaveFile, c: usize) -> Option<i16> {
	match save.objects.get(c) {
		Some(SaveObject::Complex(x)) => Some(x.id),
		_ => None,
	}
}

/// Whether complex slot `c` may serve a unit of `team`: for a player team it
/// must be in that team's list; an alien/derelict complex must be in none
/// (only the four player teams serialize a `TeamUnits` table).
fn owner_ok(save: &SaveFile, c: usize, team: usize) -> bool {
	if team < save.team_units.len() {
		save.team_units[team].complexes.contains(&c)
	} else {
		save.team_units.iter().all(|t| !t.complexes.contains(&c))
	}
}

/// Whether this host is a building still **under construction** — the one
/// state where a null complex is legitimate: the engine attaches one at
/// completion (`UnitsManager_UpdateConnectors` → `AttachToPrimaryComplex`),
/// and DOS-era stock scenarios ship saves mid-build (SAVE9.SCE's DOCK/DEPOT,
/// orders BUILD + state BUILD_CLEARING, both with null complexes). A
/// *producing factory* also sits on BUILD but in EXECUTING_ORDER, and always
/// carries its complex — the corpus confirms the discriminator.
pub(super) fn under_construction(u: &UnitRecord) -> bool {
	u.orders == ORDER_BUILD && matches!(u.state, ORDER_STATE_SELECT_SITE | ORDER_STATE_BUILD_CLEARING)
}

/// Whether a host's stored complex reference needs repair: null (except on a
/// building under construction), not a `Complex`, or outside its team's
/// domain — the states the engine dereferences or mis-indexes at run time.
fn is_orphan(save: &SaveFile, u: &UnitRecord) -> bool {
	match u.complex {
		None => !under_construction(u),
		Some(c) => complex_id(save, c).is_none() || !owner_ok(save, c, u.team as usize),
	}
}

/// Validate the complex invariant, returning one ASCII description per
/// violation (empty = nothing an export needs to repair). Read-only; the
/// export path fixes everything this reports via [`repair_complexes`].
/// Tolerated stock-data drift (high `buildings` counts, dead listed entries,
/// split clusters — see the module doc) is deliberately not reported.
pub fn check_complexes(save: &SaveFile) -> Vec<String> {
	let mut issues = Vec::new();

	// Team lists: every entry a Complex, ids >= 1, strictly ascending, and no
	// complex listed by two teams.
	let mut listed: HashMap<usize, usize> = HashMap::new();
	for (t, tu) in save.team_units.iter().enumerate() {
		let mut prev: Option<i16> = None;
		for &c in &tu.complexes {
			let Some(id) = complex_id(save, c) else {
				issues.push(format!("team {t} complex list entry {c} is not a Complex object"));
				continue;
			};
			if id < 1 {
				issues.push(format!("team {t} complex id {id} is below 1"));
			}
			if let Some(p) = prev
				&& id <= p
			{
				issues.push(format!("team {t} complex list not strictly ascending at id {id} (after {p})"));
			}
			prev = Some(id);
			if let Some(&other) = listed.get(&c) {
				issues.push(format!("complex {c} (id {id}) is listed by team {other} and team {t}"));
			}
			listed.insert(c, t);
		}
	}

	// Units: a host holds a complex owned by its own team; a non-host holds none.
	for o in &save.objects {
		let SaveObject::Unit(u) = o else { continue };
		let label = || format!("unit id {} (type {:#04x}) at {},{}", u.id, u.unit_type, u.grid_x, u.grid_y);
		if !is_complex_host(u) {
			if u.complex.is_some() {
				issues.push(format!("{} is not a complex host but references complex {:?}", label(), u.complex));
			}
		} else if u.complex.is_none() {
			if !under_construction(u) {
				issues.push(format!("{} has no complex (the engine dereferences it unguarded)", label()));
			}
		} else if is_orphan(save, u) {
			issues.push(format!("{} references complex {:?}, which is not its team's to hold", label(), u.complex));
		}
	}

	// Per complex: `buildings` at or above the real membership (below walks
	// `Shrink` to zero early and collects a live complex), and members all in
	// one connected cluster (spanning disconnected fragments is the state
	// `DetachComplex` exists to undo — an editor deletion creates it).
	let comps = components(save);
	let comp_of: HashMap<usize, usize> =
		comps.iter().enumerate().flat_map(|(ci, c)| c.members.iter().map(move |&m| (m, ci))).collect();
	let mut member_count: HashMap<usize, i16> = HashMap::new();
	let mut spans: HashMap<usize, HashSet<usize>> = HashMap::new();
	for (slot, o) in save.objects.iter().enumerate() {
		let SaveObject::Unit(u) = o else { continue };
		// An orphan's stale reference is flagged above on its own; it is not
		// membership for the count / fragment rules.
		if is_orphan(save, u) {
			continue;
		}
		let Some(c) = u.complex else { continue };
		*member_count.entry(c).or_default() += 1;
		if let Some(&ci) = comp_of.get(&slot) {
			spans.entry(c).or_default().insert(ci);
		}
	}
	for (&c, &n) in &member_count {
		if let Some(SaveObject::Complex(x)) = save.objects.get(c)
			&& x.buildings < n
		{
			issues.push(format!(
				"complex {c} (id {}) counts {} buildings but {} units reference it",
				x.id, x.buildings, n
			));
		}
		if spans.get(&c).is_some_and(|s| s.len() > 1) {
			issues.push(format!(
				"complex {c} (id {:?}) serves {} disconnected fragments",
				complex_id(save, c),
				spans[&c].len()
			));
		}
	}

	issues.sort();
	issues
}

/// The `(team, id)` of every listed complex no unit references. Computed on
/// the **pristine** save and passed to [`repair_complexes`] as its `keep_dead`,
/// so shipped dead entries (SAVE16.SCE) survive byte-identically while a
/// complex an *edit* emptied is collected like the engine's own
/// `Shrink` → `RemoveComplex` would.
pub fn dead_listed_complexes(save: &SaveFile) -> Vec<(usize, i16)> {
	let referenced: HashSet<usize> = save.units().filter_map(|u| u.complex).collect();
	let mut dead = Vec::new();
	for (t, tu) in save.team_units.iter().enumerate() {
		for &c in &tu.complexes {
			if !referenced.contains(&c)
				&& let Some(id) = complex_id(save, c)
			{
				dead.push((t, id));
			}
		}
	}
	dead
}

/// Restore the load-safety half of the complex invariant, mirroring the
/// engine's own event handlers where an edit left a hole (see the module doc
/// for the exact line between repaired and tolerated states). Returns the
/// number of repairs made — **0 for any save the engine wrote, with nothing
/// touched** (the export-identity guarantee). The five cargo fields are an
/// engine-recomputed cache (`Access_UpdateResourcesTotal` runs over every
/// complex each turn start), so a fresh complex seeds zeros.
///
/// `keep_dead` names the zero-member listed complexes the **pristine** save
/// already carried ([`dead_listed_complexes`]) — those are tolerated drift and
/// stay; any other complex left memberless is collected from the graph and its
/// team list, like the engine's `Shrink` → `RemoveComplex`.
///
/// `Err` only if a created/removed `Complex` cannot move the save's tail with
/// it ([`SaveFile::insert_object`] / [`SaveFile::remove_object`]) — and then
/// the save may hold a partial repair; callers treat the export as failed.
pub fn repair_complexes(save: &mut SaveFile, keep_dead: &[(usize, i16)]) -> Result<usize, EditError> {
	let mut changes = 0usize;

	// A complex reference on a non-host is engine-impossible: scrub it.
	for o in &mut save.objects {
		if let SaveObject::Unit(u) = o
			&& u.complex.is_some()
			&& !is_complex_host(u)
		{
			u.complex = None;
			changes += 1;
		}
	}

	// Team lists sorted ascending by id (CreateComplex's gap scan assumes it),
	// with duplicate / sub-1 ids reassigned to the lowest free id.
	for t in 0..save.team_units.len() {
		let ids = |save: &SaveFile, t: usize| -> Vec<i16> {
			save.team_units[t].complexes.iter().filter_map(|&c| complex_id(save, c)).collect()
		};
		let sorted = |v: &[i16]| v.windows(2).all(|w| w[0] < w[1]);
		if !sorted(&ids(save, t)) {
			let mut list = save.team_units[t].complexes.clone();
			list.sort_by_key(|&c| complex_id(save, c));
			save.team_units[t].complexes = list;
			changes += 1;
		}
		// After sorting, an invalid id is either < 1 or equal to its predecessor.
		loop {
			let cur = ids(save, t);
			let bad = (0..cur.len()).find(|&i| cur[i] < 1 || (i > 0 && cur[i] == cur[i - 1]));
			let Some(i) = bad else { break };
			let free = (1..).find(|id| !cur.contains(id)).expect("i16 id space has a free slot");
			let c = save.team_units[t].complexes[i];
			if let Some(SaveObject::Complex(x)) = save.objects.get_mut(c) {
				x.id = free;
				let body = serialize_complex(x);
				save.object_meta[c].body_raw = body;
			}
			let mut list = save.team_units[t].complexes.clone();
			list.sort_by_key(|&c| complex_id(save, c));
			save.team_units[t].complexes = list;
			changes += 1;
		}
	}

	// Complexes whose membership the surgery changes: their `buildings` is set
	// to the exact new count at the end (the engine's Grow/Shrink would have).
	let mut touched: HashSet<usize> = HashSet::new();

	// `DetachComplex` mirror: a complex serving disconnected fragments keeps its
	// first fragment; every later fragment gets a fresh complex, flooded along
	// the fragment's own connector bits (`TestConnections` + `AttachComplex`).
	let mut comps = components(save);
	loop {
		let comp_of: HashMap<usize, usize> =
			comps.iter().enumerate().flat_map(|(ci, c)| c.members.iter().map(move |&m| (m, ci))).collect();
		let mut spans: HashMap<usize, Vec<usize>> = HashMap::new();
		for (slot, o) in save.objects.iter().enumerate() {
			let SaveObject::Unit(u) = o else { continue };
			// An orphan's stale reference (e.g. a re-teamed building still
			// pointing at its old team's complex) is re-homed by the orphan
			// stage below — it must not hold a fragment here.
			if is_orphan(save, u) {
				continue;
			}
			let (Some(c), Some(&ci)) = (u.complex, comp_of.get(&slot)) else { continue };
			let span = spans.entry(c).or_default();
			if !span.contains(&ci) {
				span.push(ci);
			}
		}
		// The first fragment (in walk order) keeps the complex; re-home the
		// earliest member of the earliest *later* fragment, then re-derive —
		// exactly the engine's one-fragment-at-a-time do-while.
		let Some((&c, span)) = spans.iter().filter(|(_, s)| s.len() > 1).min_by_key(|&(&c, _)| c) else { break };
		let mut span = span.clone();
		span.sort_unstable();
		let ci = span[1];
		let seed = *comps[ci].members.iter().find(|&&m| save.unit(m).is_some_and(|u| u.complex == Some(c))).unwrap();
		let team = comps[ci].team as usize;
		let occ = occupancy(save);
		let reach = attach_reach(save, &occ, seed);
		let at = create_complex(save, team, &reach)?;
		shift_up(&mut comps, &mut touched, at);
		changes += 1;
		touched.insert(at);
		for m in reach.into_iter().map(|m| if m >= at { m + 1 } else { m }) {
			repoint(save, m, at, &mut touched, &mut changes);
		}
	}

	// `AttachToPrimaryComplex` mirror, per orphaned host (a placed building, a
	// team-edited one, or a neighbour a bridge should merge): the lowest-id
	// complex among the units its own bits connect to wins — else a fresh one —
	// and the `AttachComplex` flood re-points everything reachable, which is
	// also what merges two bridged complexes into the lower id.
	loop {
		let orphan =
			(0..save.objects.len()).find(|&s| save.unit(s).is_some_and(|u| is_complex_host(u) && is_orphan(save, u)));
		let Some(mut seed) = orphan else { break };
		let team = save.unit(seed).expect("orphan is a unit").team as usize;
		let occ = occupancy(save);
		let winner = connected_neighbors(save, &occ, seed)
			.into_iter()
			.filter_map(|n| save.unit(n).and_then(|v| v.complex))
			.filter(|&c| complex_id(save, c).is_some() && owner_ok(save, c, team))
			.min_by_key(|&c| complex_id(save, c));
		let winner = match winner {
			Some(w) => w,
			None => {
				let reach = attach_reach(save, &occ, seed);
				let at = create_complex(save, team, &reach)?;
				shift_up(&mut comps, &mut touched, at);
				if seed >= at {
					seed += 1;
				}
				changes += 1;
				at
			}
		};
		touched.insert(winner);
		let occ = occupancy(save);
		for m in attach_reach(save, &occ, seed) {
			repoint(save, m, winner, &mut touched, &mut changes);
		}
	}

	// Collect memberless complexes (the engine's Shrink-to-zero →
	// `RemoveComplex`; reference cleanup also drops the team-list entry) —
	// except the dead listed entries the pristine save already carried, which
	// are tolerated drift and must stay byte-identical (SAVE16.SCE).
	loop {
		let referenced: HashSet<usize> = save.units().filter_map(|u| u.complex).collect();
		let empty = (0..save.objects.len()).find(|&c| {
			if !matches!(save.objects.get(c), Some(SaveObject::Complex(_))) || referenced.contains(&c) {
				return false;
			}
			let carried_over = save.team_units.iter().enumerate().any(|(t, tu)| {
				tu.complexes.contains(&c) && complex_id(save, c).is_some_and(|id| keep_dead.contains(&(t, id)))
			});
			!carried_over
		});
		let Some(c) = empty else { break };
		save.remove_object(c)?;
		touched.remove(&c);
		touched = touched.iter().map(|&x| if x > c { x - 1 } else { x }).collect();
		for comp in &mut comps {
			for m in &mut comp.members {
				if *m > c {
					*m -= 1;
				}
			}
		}
		changes += 1;
	}

	// `buildings`: exact for complexes whose membership the surgery changed
	// (Grow/Shrink would have tracked it); raised when below the membership on
	// the rest (the early-collection hazard); never lowered otherwise — stock
	// data ships counts that run high (SAVE3.SCE) and the engine tolerates it.
	let mut counts: HashMap<usize, i16> = HashMap::new();
	for u in save.units() {
		if let Some(c) = u.complex {
			*counts.entry(c).or_default() += 1;
		}
	}
	for (c, n) in counts {
		if let Some(SaveObject::Complex(x)) = save.objects.get_mut(c)
			&& ((touched.contains(&c) && x.buildings != n) || x.buildings < n)
		{
			x.buildings = n;
			let body = serialize_complex(x);
			save.object_meta[c].body_raw = body;
			changes += 1;
		}
	}

	Ok(changes)
}

/// Point unit `m` at complex `winner`, marking both the complex it leaves and
/// the winner as membership-touched.
fn repoint(save: &mut SaveFile, m: usize, winner: usize, touched: &mut HashSet<usize>, changes: &mut usize) {
	let old = save.unit(m).and_then(|u| u.complex);
	if old == Some(winner) {
		return;
	}
	if let Some(o) = old {
		touched.insert(o);
	}
	if let Some(SaveObject::Unit(u)) = save.objects.get_mut(m) {
		u.complex = Some(winner);
		*changes += 1;
	}
}

/// Shift every local slot record at/above a just-inserted object index up by
/// one, mirroring what [`SaveFile::insert_object`] did to the graph.
fn shift_up(comps: &mut [Component], touched: &mut HashSet<usize>, at: usize) {
	for comp in comps.iter_mut() {
		for m in &mut comp.members {
			if *m >= at {
				*m += 1;
			}
		}
	}
	*touched = touched.iter().map(|&c| if c >= at { c + 1 } else { c }).collect();
}

/// Create a fresh `Complex` for the member slots in `members` (a flood's reach)
/// and wire it into the graph — and, for a player team, the sorted team list —
/// at its first-seen position. Returns the new object's slot.
fn create_complex(save: &mut SaveFile, team: usize, members: &[usize]) -> Result<usize, EditError> {
	let (id, at, list_pos) = if team < save.team_units.len() {
		// CreateComplex's gap scan over the sorted list: the lowest free id >= 1,
		// which is also the list position to insert at.
		let list = &save.team_units[team].complexes;
		let mut id: i16 = 1;
		let mut k = 0usize;
		while k < list.len() && complex_id(save, list[k]) == Some(id) {
			id += 1;
			k += 1;
		}
		// Region 19 emits the four team tables in order, each as base values,
		// current values, then the complex list sorted by id — so the new complex
		// is first-seen right after every object those regions emit before it.
		let mut max_idx: Option<usize> = None;
		let mut bump = |i: usize| max_idx = Some(max_idx.map_or(i, |m: usize| m.max(i)));
		for (t, tu) in save.team_units.iter().enumerate().take(team + 1) {
			for &r in tu.base_values.iter().chain(tu.current_values.iter()).flatten() {
				bump(r);
			}
			let before = if t < team { &tu.complexes[..] } else { &tu.complexes[..k] };
			for &c in before {
				bump(c);
			}
		}
		(id, max_idx.map_or(0, |m| m + 1), Some(k))
	} else {
		// Alien/derelict: no serialized team list. The complex is first-seen at
		// the first member's own `complex` reference — after that unit and the
		// inline leaves emitted before the reference (`path`, `base_values`).
		let ids: HashSet<i16> = (0..save.objects.len())
			.filter(|&c| save.team_units.iter().all(|t| !t.complexes.contains(&c)))
			.filter_map(|c| complex_id(save, c))
			.collect();
		let id = (1..).find(|i| !ids.contains(i)).expect("i16 id space has a free slot");
		// Walk order visits the five unit lists in on-disk order; the first
		// member a list carries is the one whose reference emits first.
		let m = save
			.lists()
			.into_iter()
			.flat_map(|(_, list)| list.iter().copied())
			.find(|s| members.contains(s))
			.unwrap_or(members[0]);
		let u = save.unit(m).expect("member slot holds a unit");
		let inline = |r: Option<usize>| r.is_some_and(|p| p > m) as usize;
		(id, m + 1 + inline(u.path) + inline(u.base_values), None)
	};

	let c = Complex { material: 0, fuel: 0, gold: 0, power: 0, workers: 0, buildings: members.len() as i16, id };
	let meta = ObjMeta { type_index: COMPLEX_TYPE, contained: 1, body_raw: serialize_complex(&c), unit_layout: None };
	save.insert_object(at, SaveObject::Complex(c), meta)?;
	if let Some(k) = list_pos {
		save.team_units[team].complexes.insert(k, at);
	}
	Ok(at)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::save::{add_unit, read_save, remove_unit, write_save};

	/// Every stock save passes the checker as modeled here, the repair is a
	/// no-op on all of them, and the file still round-trips byte-identically
	/// afterwards — the export-identity guarantee, and the proof that the
	/// line between repaired and tolerated states matches what the engine
	/// actually loads (the module-doc table).
	#[test]
	fn stock_saves_satisfy_the_invariant_and_repair_is_a_noop() {
		let Some(home) = std::env::var_os("HOME") else { return };
		let max_dir = std::path::Path::new(&home).join("MAX");
		let originals = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals");
		if !max_dir.is_dir() || !originals.is_dir() {
			crate::testutil::skip_fixture("stock_saves_satisfy_the_invariant_and_repair_is_a_noop: fixtures not found");
			return;
		}
		let mut checked = 0;
		for entry in std::fs::read_dir(&max_dir).into_iter().flatten().flatten() {
			let path = entry.path();
			let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_uppercase);
			if !matches!(ext.as_deref(), Some("DTA" | "CAM" | "SCE" | "TRA" | "MPS" | "DMO")) {
				continue;
			}
			let Ok(header) = crate::save::read_save_header(&path) else { continue };
			let Some(world_file) = header.world_file else { continue };
			let Ok(wrl) = crate::wrl::read_wrl_header(&originals.join(world_file)) else { continue };
			let Ok(save) = read_save(&path, (wrl.width, wrl.height)) else { continue };

			let issues = check_complexes(&save);
			assert!(issues.is_empty(), "{}: invariant violated:\n  {}", path.display(), issues.join("\n  "));

			let original = std::fs::read(&path).unwrap();
			let mut repaired = save.clone();
			let keep = dead_listed_complexes(&save);
			let changes = repair_complexes(&mut repaired, &keep).expect("repair walks");
			assert_eq!(changes, 0, "{}: repair must not touch a valid save", path.display());
			assert!(write_save(&repaired).unwrap() == original, "{}: repair broke byte identity", path.display());
			checked += 1;
		}
		assert!(checked > 0, "no ~/MAX saves were checked");
		eprintln!("complex invariant holds on {checked} stock saves; repair untouched all of them");
	}

	/// Locate SAVE10 (V70) or skip when absent.
	fn save10() -> Option<SaveFile> {
		let home = std::env::var_os("HOME")?;
		let save_path = std::path::Path::new(&home).join("MAX/SAVE10.DTA");
		let wrl_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/originals/SNOW_1.WRL");
		if !save_path.is_file() || !wrl_path.is_file() {
			return None;
		}
		let wrl = crate::wrl::read_wrl_header(&wrl_path).ok()?;
		read_save(&save_path, (wrl.width, wrl.height)).ok()
	}

	/// `ResourceID` of the mining station used as the test building (present in
	/// SAVE10, 2x2, a connector host).
	const MININGST: u16 = 0x28;

	/// Place a MININGST for team 0 at `(x, y)` and give it connector `mask`.
	/// Returns the new unit's spatial-hash id.
	fn place(save: &mut SaveFile, x: u16, y: u16, mask: u16) -> u16 {
		let id = add_unit(save, MININGST, 0, x, y, None).expect("tail follows").expect("template exists");
		let slot = save.objects.iter().position(|o| matches!(o, SaveObject::Unit(u) if u.id == id)).unwrap();
		if let Some(SaveObject::Unit(u)) = save.objects.get_mut(slot) {
			u.connectors = mask;
		}
		id
	}

	/// The complex slot a unit references, resolved by unit id.
	fn complex_of(save: &SaveFile, id: u16) -> Option<usize> {
		save.units().find(|u| u.id == id).and_then(|u| u.complex)
	}

	/// The repaired file re-decodes, passes the checker, and its own re-export
	/// is stable — run after every scenario below.
	fn assert_valid(save: &SaveFile) {
		let bytes = write_save(save).unwrap();
		let redecoded = crate::save::read_save_bytes(&bytes, (save.width, save.height)).expect("re-decode");
		let issues = check_complexes(&redecoded);
		assert!(issues.is_empty(), "exported save violates the invariant:\n  {}", issues.join("\n  "));
		assert!(write_save(&redecoded).unwrap() == bytes, "exported save must round-trip byte-identically");
	}

	/// A placed building — the Finding 1 defect — gets a fresh complex: listed
	/// by its team at the lowest free id, `buildings == 1`.
	#[test]
	fn a_placed_building_gets_its_own_complex() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("a_placed_building_gets_its_own_complex: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		let listed_before = save.team_units[0].complexes.len();
		let id = place(&mut save, 40, 40, 0);
		assert!(!check_complexes(&save).is_empty(), "the null complex is detected before repair");

		let changes = repair_complexes(&mut save, &keep).expect("repair walks");
		assert!(changes > 0, "the placement needed repair");
		let c = complex_of(&save, id).expect("the placed building has a complex now");
		assert!(save.team_units[0].complexes.contains(&c), "listed by its own team");
		assert_eq!(save.team_units[0].complexes.len(), listed_before + 1, "exactly one new complex");
		let Some(SaveObject::Complex(x)) = save.objects.get(c) else { panic!("complex object") };
		assert_eq!(x.buildings, 1, "it counts its one member");
		let ids: Vec<i16> = save.team_units[0].complexes.iter().filter_map(|&c| complex_id(&save, c)).collect();
		assert!(ids.windows(2).all(|w| w[0] < w[1]), "team list stays sorted: {ids:?}");
		assert_valid(&save);
	}

	/// Two adjacent connected placements share ONE complex with
	/// `buildings == 2`; two placements apart get two complexes, distinct ids.
	#[test]
	fn adjacency_decides_shared_versus_separate_complexes() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("adjacency_decides_shared_versus_separate_complexes: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		// Adjacent pair: A at (40,40), B at (42,40) — B's west bits point at A.
		let a = place(&mut save, 40, 40, 0x04 | 0x08); // ET | EB -> (42,40),(42,41)
		let b = place(&mut save, 42, 40, 0x40 | 0x80); // WT | WB -> (41,40),(41,41)
		// Apart: C at (50, 50), no connections.
		let c = place(&mut save, 50, 50, 0);

		repair_complexes(&mut save, &keep).expect("repair walks");
		let (ca, cb, cc) = (complex_of(&save, a), complex_of(&save, b), complex_of(&save, c));
		assert_eq!(ca, cb, "adjacent connected buildings share one complex");
		assert_ne!(ca, cc, "the distant building has its own");
		let Some(SaveObject::Complex(x)) = save.objects.get(ca.unwrap()) else { panic!("complex object") };
		assert_eq!(x.buildings, 2, "the shared complex counts both members");
		let Some(SaveObject::Complex(y)) = save.objects.get(cc.unwrap()) else { panic!("complex object") };
		assert_eq!(y.buildings, 1);
		assert_ne!(x.id, y.id, "distinct ids");
		assert_valid(&save);
	}

	/// A building placed to bridge two existing complexes merges them into the
	/// LOWER id; the higher-id `Complex` leaves both the graph and the list.
	#[test]
	fn a_bridge_placement_merges_into_the_lower_id() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("a_bridge_placement_merges_into_the_lower_id: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		let listed_before = save.team_units[0].complexes.len();
		// Two separate stations...
		let a = place(&mut save, 40, 40, 0);
		let c = place(&mut save, 44, 40, 0);
		repair_complexes(&mut save, &keep).expect("repair walks");
		let (ca, cc) = (complex_of(&save, a).unwrap(), complex_of(&save, c).unwrap());
		assert_ne!(ca, cc);
		let (ida, idc) = (complex_id(&save, ca).unwrap(), complex_id(&save, cc).unwrap());
		assert!(ida < idc, "first placement got the lower id");

		// ...bridged by a third whose own bits reach both.
		let b = place(&mut save, 42, 40, 0x40 | 0x80 | 0x04 | 0x08); // WT|WB|ET|EB
		repair_complexes(&mut save, &keep).expect("repair walks");
		let merged = complex_of(&save, b).expect("bridge attached");
		assert_eq!(complex_of(&save, a), Some(merged), "A re-pointed to the winner");
		assert_eq!(complex_of(&save, c), Some(merged), "C re-pointed to the winner");
		assert_eq!(complex_id(&save, merged), Some(ida), "the LOWER id won");
		let Some(SaveObject::Complex(x)) = save.objects.get(merged) else { panic!("complex object") };
		assert_eq!(x.buildings, 3, "the merged complex counts all three");
		assert_eq!(save.team_units[0].complexes.len(), listed_before + 1, "the losing complex left the list");
		let ids: Vec<i16> = save.team_units[0].complexes.iter().filter_map(|&s| complex_id(&save, s)).collect();
		assert!(!ids.contains(&idc), "the losing id is gone");
		assert_valid(&save);
	}

	/// Deleting the middle of a connected chain splits the complex: the first
	/// fragment keeps it, the split-off one gets a fresh complex.
	#[test]
	fn deleting_the_middle_splits_the_complex() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("deleting_the_middle_splits_the_complex: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		// A chain A-B-C where only B carries connector bits (both directions).
		let a = place(&mut save, 40, 40, 0);
		let c = place(&mut save, 44, 40, 0);
		let b = place(&mut save, 42, 40, 0x40 | 0x80 | 0x04 | 0x08);
		repair_complexes(&mut save, &keep).expect("repair walks");
		let shared = complex_of(&save, a).unwrap();
		assert_eq!(complex_of(&save, c), Some(shared), "one complex before the cut");

		assert!(remove_unit(&mut save, b).expect("tail follows"), "middle removed");
		repair_complexes(&mut save, &keep).expect("repair walks");
		let (ca, cc) = (complex_of(&save, a).unwrap(), complex_of(&save, c).unwrap());
		assert_ne!(ca, cc, "the fragments no longer share a complex");
		for s in [ca, cc] {
			let Some(SaveObject::Complex(x)) = save.objects.get(s) else { panic!("complex object") };
			assert_eq!(x.buildings, 1, "each fragment counts its own member");
		}
		assert_valid(&save);
	}

	/// Deleting a complex's last member collects the complex from both the
	/// graph and the team list — while a dead entry the pristine save already
	/// carried would be kept (`keep_dead`, exercised by the corpus test).
	#[test]
	fn deleting_the_last_member_collects_the_complex() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("deleting_the_last_member_collects_the_complex: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		let listed_before = save.team_units[0].complexes.len();
		let complexes_before = save.objects.iter().filter(|o| matches!(o, SaveObject::Complex(_))).count();
		let id = place(&mut save, 40, 40, 0);
		repair_complexes(&mut save, &keep).expect("repair walks");
		assert_eq!(save.team_units[0].complexes.len(), listed_before + 1);

		assert!(remove_unit(&mut save, id).expect("tail follows"));
		repair_complexes(&mut save, &keep).expect("repair walks");
		assert_eq!(save.team_units[0].complexes.len(), listed_before, "the list entry is gone");
		let complexes_after = save.objects.iter().filter(|o| matches!(o, SaveObject::Complex(_))).count();
		assert_eq!(complexes_after, complexes_before, "the Complex object is gone from the graph");
		assert_valid(&save);
	}

	/// An alien/derelict placement gets an UNLISTED complex (only the four
	/// player teams serialize a `TeamUnits` table) that still round-trips.
	#[test]
	fn an_alien_placement_gets_an_unlisted_complex() {
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("an_alien_placement_gets_an_unlisted_complex: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		let id = add_unit(&mut save, MININGST, 4, 40, 40, None).expect("tail follows").expect("template exists");
		repair_complexes(&mut save, &keep).expect("repair walks");
		let c = complex_of(&save, id).expect("the derelict host has a complex");
		assert!(save.team_units.iter().all(|t| !t.complexes.contains(&c)), "listed by no player team");
		let Some(SaveObject::Complex(x)) = save.objects.get(c) else { panic!("complex object") };
		assert_eq!(x.buildings, 1);
		assert_valid(&save);
	}

	/// A team edit — through the real export path, `patch_unit_scalars` — moves
	/// a building out of its old complex: the building gets a complex of its
	/// new team, and the drained complex's count follows the exact membership
	/// the surgery left behind.
	#[test]
	fn a_team_edit_rehomes_the_building() {
		use crate::save::{UnitScalarEdit, patch_unit_scalars};
		let Some(mut save) = save10() else {
			crate::testutil::skip_fixture("a_team_edit_rehomes_the_building: SAVE10 fixture absent");
			return;
		};
		let keep = dead_listed_complexes(&save);
		// An existing team-0 host whose complex has other members too (so the
		// drained complex survives the edit), from the pristine save.
		let u0 = save
			.units()
			.find(|u| {
				u.team == 0
					&& is_complex_host(u)
					&& u.complex.is_some_and(|c| save.units().filter(|v| v.complex == Some(c)).count() >= 2)
			})
			.expect("SAVE10 has a team-0 host in a multi-member complex")
			.clone();
		let old_id = complex_id(&save, u0.complex.unwrap()).unwrap();
		let old_buildings = match &save.objects[u0.complex.unwrap()] {
			SaveObject::Complex(x) => x.buildings,
			_ => unreachable!(),
		};
		let edit = UnitScalarEdit {
			id: u0.id,
			team: 1,
			name: &u0.name,
			angle: u0.angle,
			turret_angle: u0.turret_angle,
			hits: u0.hits,
			ammo: u0.ammo,
			orders: u0.orders,
			disabled_turns: u0.disabled_turns,
			storage: u0.storage,
			connectors: 0, // a re-teamed building leaves the old connector grid
		};
		assert!(patch_unit_scalars(&mut save, &edit));

		repair_complexes(&mut save, &keep).expect("repair walks");
		let got = save.units().find(|u| u.id == u0.id).unwrap();
		assert_eq!(got.team, 1, "the record carries the new team");
		let c = got.complex.expect("still has a complex");
		assert!(save.team_units[1].complexes.contains(&c), "listed by the NEW team");
		// The drained old complex, re-resolved by id (repair shifts slots).
		let old_slot = save.team_units[0]
			.complexes
			.iter()
			.copied()
			.find(|&s| complex_id(&save, s) == Some(old_id))
			.expect("the old complex still exists (it kept other members)");
		assert_ne!(c, old_slot, "no longer the old team's complex");
		let members = save.units().filter(|u| u.complex == Some(old_slot)).count();
		let Some(SaveObject::Complex(x)) = save.objects.get(old_slot) else { panic!("complex object") };
		assert_eq!(x.buildings as usize, members, "the drained complex counts its remaining members");
		assert!(x.buildings < old_buildings, "one fewer than before");
		assert_valid(&save);
	}
}
