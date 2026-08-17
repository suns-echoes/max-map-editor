//! Dump the header — and, when the world can be resolved, the full unit
//! inventory — of one or more M.A.X. save files.
//!
//! Usage: `cargo run -p max-assets --example dump_save -- <SAVE#.DTA> [more...]`
//!
//! The save file does not store map dimensions, so a full decode needs the
//! pristine `.WRL` the save references. This example looks for it under
//! `testdata/originals/` (the bundled pristine stock maps); a `MME_WRL_DIR`
//! environment variable overrides the directory. If the world isn't found, only
//! the header is printed.

use std::path::{Path, PathBuf};

use max_assets::save::{SaveFile, SaveHeader, TEAM_LABELS, read_save, read_save_header, unit_type_name};
use max_assets::wrl::read_wrl_header;

fn main() {
	let paths: Vec<String> = std::env::args().skip(1).collect();
	if paths.is_empty() {
		eprintln!("usage: dump_save <save-file> [more save-files...]");
		std::process::exit(2);
	}

	let mut failures = 0;
	for path in &paths {
		println!("== {path} ==");
		match read_save_header(Path::new(path)) {
			Ok(header) => {
				print_header(&header);
				dump_body(path, &header);
			}
			Err(err) => {
				failures += 1;
				println!("  ERROR: {err}");
			}
		}
		println!();
	}

	if failures > 0 {
		std::process::exit(1);
	}
}

/// Attempts a full decode once the referenced world's dimensions are known.
fn dump_body(path: &str, header: &SaveHeader) {
	let Some(world_file) = header.world_file else {
		println!("  inventory   : (world not resolvable — header only)");
		return;
	};
	let Some(wrl_path) = find_wrl(world_file) else {
		println!("  inventory   : (no pristine {world_file} found — set MME_WRL_DIR; header only)");
		return;
	};
	let dims = match read_wrl_header(&wrl_path) {
		Ok(h) => (h.width, h.height),
		Err(err) => {
			println!("  inventory   : (failed to read {}: {err})", wrl_path.display());
			return;
		}
	};

	match read_save(Path::new(path), dims) {
		Ok(save) => print_inventory(&save, &wrl_path, dims),
		Err(err) => println!("  inventory   : DECODE ERROR: {err}"),
	}
}

fn find_wrl(world_file: &str) -> Option<PathBuf> {
	let mut dirs: Vec<PathBuf> = Vec::new();
	if let Ok(dir) = std::env::var("MME_WRL_DIR") {
		dirs.push(PathBuf::from(dir));
	}
	dirs.push(PathBuf::from("testdata/originals"));
	dirs.push(PathBuf::from("crates/max-assets/testdata/originals"));
	for dir in dirs {
		let candidate = dir.join(world_file);
		if candidate.is_file() {
			return Some(candidate);
		}
	}
	None
}

fn print_inventory(save: &SaveFile, wrl_path: &Path, dims: (u16, u16)) {
	println!("  world map   : {} ({}x{})", wrl_path.display(), dims.0, dims.1);
	println!(
		"  game state  : active_team={} player_team={} turn={} game_state={} turn_timer={}",
		save.active_turn_team, save.player_team, save.turn_counter, save.game_state, save.turn_timer
	);

	let surveyed = save.cargo_map.iter().filter(|&&v| v != 0).count();
	let raw_total: u64 = save.cargo_map.iter().map(|&v| v as u64).sum();
	println!("  resources   : {surveyed} surveyed cells (total cargo {raw_total})");

	let total_objects = save.objects.len();
	let total_units: usize = save.lists().iter().map(|(_, l)| l.len()).sum();
	println!("  objects     : {total_objects} in graph; {total_units} units across 5 lists");

	for (name, list) in save.lists() {
		if list.is_empty() {
			continue;
		}
		println!("  {name:<15}: {} unit(s)", list.len());
		// Tally unit types within this list.
		let mut by_type: Vec<(u16, u32, usize)> = Vec::new(); // (unit_type, team, count) folded by (type,team)
		for &idx in list {
			let Some(u) = save.unit(idx) else { continue };
			if let Some(entry) = by_type.iter_mut().find(|(t, tm, _)| *t == u.unit_type && *tm == u.team as u32) {
				entry.2 += 1;
			} else {
				by_type.push((u.unit_type, u.team as u32, 1));
			}
		}
		by_type.sort_by_key(|&(t, tm, _)| (t, tm));
		for (unit_type, team, count) in by_type {
			let name = unit_type_name(unit_type).unwrap_or("?");
			println!("      {name:<8} ({unit_type:#04x})  team {team}  x{count}");
		}
	}

	// Spot-check a few units with their base-values (upgraded max HP etc.).
	println!("  sample units:");
	for u in save.units().take(6) {
		let base = u.base_values.and_then(|i| save.values(i));
		let max_hits = base.map(|v| v.hits).unwrap_or(0);
		let kind = unit_type_name(u.unit_type).unwrap_or("?");
		let name = if u.name.is_empty() { String::new() } else { format!(" {:?}", u.name) };
		println!(
			"      {kind:<8} ({:#04x}) id={} team={} at ({},{}) hits={}/{} ammo={} orders={}{}",
			u.unit_type, u.id, u.team, u.grid_x, u.grid_y, u.hits, max_hits, u.ammo, u.orders, name
		);
	}
}

fn print_header(h: &SaveHeader) {
	let world = match (h.world_file, h.world_index) {
		(Some(file), Some(idx)) => format!("{file} (index {idx})"),
		(None, Some(idx)) => format!("index {idx} (out of stock range)"),
		_ => match &h.world_hash {
			Some(hash) => format!("custom, hash {hash}"),
			None => "unknown".to_string(),
		},
	};

	println!("  format      : {:?} (v{})", h.format, h.format.version());
	println!("  category    : {}", h.category.label());
	println!("  save name   : {:?}", h.save_name);
	println!("  world       : {world}");
	println!("  rng seed    : {:#010x}", h.rng_seed);
	if h.mission_index != 0 {
		println!("  mission idx : {}", h.mission_index);
	}
	if !h.script.is_empty() {
		println!("  script      : {} bytes", h.script.len());
	}

	print!("  teams       :");
	let mut any = false;
	for (i, team_name) in h.team_names.iter().enumerate() {
		// team_type 0 == none; skip empty non-participating slots.
		if h.team_type[i] == 0 && team_name.is_empty() {
			continue;
		}
		any = true;
		let name = if team_name.is_empty() { "-" } else { team_name.as_str() };
		print!(" [{} {:?} type={} clan={}]", TEAM_LABELS[i], name, h.team_type[i], h.team_clan[i]);
	}
	if !any {
		print!(" (none)");
	}
	println!();

	let o = &h.options;
	println!(
		"  options     : world={} timer={} endturn={} start_gold={} play_mode={} opponent={}",
		o.world, o.timer, o.endturn, o.start_gold, o.play_mode, o.opponent
	);
	println!(
		"                victory_type={} victory_limit={} raw={} fuel={} gold={} alien_derelicts={}",
		o.victory_type, o.victory_limit, o.raw_resource, o.fuel_resource, o.gold_resource, o.alien_derelicts
	);
}
