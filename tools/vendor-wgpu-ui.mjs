#!/usr/bin/env node
// wgpu-ui sync tool - no npm dependencies, plain Node.
//
//   node tools/vendor-wgpu-ui.mjs           push: crates/wgpu-ui -> the sibling
//                                           checkout (the default direction)
//   node tools/vendor-wgpu-ui.mjs --pull    pull: the sibling -> crates/wgpu-ui,
//                                           for work another agent did there
//   node tools/vendor-wgpu-ui.mjs --check   report status, change nothing
//                                           (exit 1 on anything unsynced)
//   ... --force                             with push/pull: resolve conflicts in
//                                           favour of the pushed/pulled side
//
// WHICH COPY YOU EDIT: `crates/wgpu-ui`, here, committed to this repo. It is
// what the path dep builds, what CI and release builds compile, and what a fresh
// clone gets - `wgpu-ui` has no public remote, so a clone has no sibling to
// build against. The sibling at WGPU_UI_SRC is the shared working checkout that
// carries the toolkit's own test suite (`tests/` is not mirrored) and is where
// OTHER projects consume it from. So: edit here, `--push` there, run the
// toolkit's tests there.
//
// CONFLICTS ARE REAL: another agent, in another project, edits that sibling too.
// This tool is three-way, not a blind copy - `tools/wgpu-ui-sync.json` records
// the hash of every mirrored file as of the last time the two sides agreed, and
// each side is compared against that baseline:
//
//   both match            in sync, nothing to do
//   only here changed     PUSH   (a plain copy out; refused by --pull)
//   only the sibling      INCOMING - run --pull to take it (refused by a push)
//   both changed          CONFLICT - resolve by hand, or --force one side
//
// A push therefore cannot silently bury the other agent's work, and a pull
// cannot silently revert yours.
//
// Cargo.toml and README.md under crates/wgpu-ui are GENERATED here, not synced:
// the upstream manifest inherits version/edition/deps from the wgpu-ui
// workspace and this one has to inherit them from ours instead. They are
// rewritten on every push/pull and drift-checked by --check. A toolkit
// dependency or pin change is therefore still made in the SIBLING's Cargo.toml
// (and mirrored into ours by hand) - it is the only thing that does not flow
// out of this copy.

import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, unlinkSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const CHECK = args.includes("--check");
const PULL = args.includes("--pull");
const FORCE = args.includes("--force");
const unknown = args.filter((a) => !["--check", "--pull", "--force"].includes(a));
if (unknown.length) {
	console.error(`vendor-wgpu-ui: unknown argument(s): ${unknown.join(" ")}`);
	console.error("vendor-wgpu-ui: usage: vendor-wgpu-ui.mjs [--check | --pull] [--force]");
	process.exit(2);
}
if (CHECK && (PULL || FORCE)) {
	console.error("vendor-wgpu-ui: --check changes nothing, so it takes no --pull/--force");
	process.exit(2);
}

// The sibling checkout. Overridable so a runner (or a differently-laid-out
// working copy) can point at one without editing this file.
const SRC = resolve(ROOT, process.env.WGPU_UI_SRC ?? "../../../Rust/wgpu-ui");
const SRC_CRATE = join(SRC, "crates/wgpu-ui");
const DEST = join(ROOT, "crates/wgpu-ui");
const BASELINE = join(ROOT, "tools/wgpu-ui-sync.json");

// Synced verbatim. `assets` is not optional: src/ `include_bytes!`s the font.
const MIRRORED = ["src", "assets"];
// Also synced verbatim, but living at the sibling WORKSPACE root rather than in
// its crate: rustfmt resolves the nearest config to each file, so keeping
// wgpu-ui's own rustfmt.toml beside the vendored source makes `cargo fmt` here
// reproduce upstream's style (4 spaces) instead of this repo's (hard tabs, 120
// cols) - which would rewrite every line and drown every sync in noise.
const MIRRORED_ROOT_FILES = ["rustfmt.toml"];
// The pins both workspaces must agree on, exactly - two exact pins of one
// crate cannot coexist, and a mismatch builds two copies whose boundary types
// stop unifying ("expected `wgpu::Device`, found `wgpu::Device`").
const SHARED = ["wgpu", "bytemuck", "winit"];

/** `[workspace.dependencies]` of a Cargo.toml as name -> version string.
 *  Line-based on purpose: both manifests are first-party and keep one
 *  dependency per line; anything fancier should fail loudly and be read. */
function workspaceDeps(tomlPath) {
	const deps = new Map();
	let inSection = false;
	for (const raw of readFileSync(tomlPath, "utf8").split("\n")) {
		const line = raw.trim();
		if (line.startsWith("[")) {
			inSection = line === "[workspace.dependencies]";
			continue;
		}
		if (!inSection || line === "" || line.startsWith("#")) continue;
		const m = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/);
		if (!m) continue;
		const [, name, rhs] = m;
		deps.set(name, rhs.match(/version\s*=\s*"([^"]+)"/)?.[1] ?? rhs.match(/^"([^"]+)"/)?.[1]);
	}
	return deps;
}

/** Every file under `dir`, as `/`-joined paths relative to it, sorted. */
function filesUnder(dir) {
	if (!existsSync(dir)) return [];
	return readdirSync(dir, { recursive: true, withFileTypes: true })
		.filter((e) => e.isFile())
		.map((e) => relative(dir, join(e.parentPath ?? e.path, e.name)).split("\\").join("/"))
		.sort();
}

/** Content hash, or `null` for a file that is not there - the two states each
 *  side of a comparison can be in. */
function hashOf(path) {
	if (!existsSync(path)) return null;
	return createHash("sha256").update(readFileSync(path)).digest("hex");
}

/** The vendored manifest. Deps and package metadata inherit from OUR
 *  workspace, so the pins cannot drift from the editor's by construction.
 *  The lints are spelled out rather than inherited: this crate is held to
 *  wgpu-ui's stricter policy (`unsafe_code = "deny"`), not the app's
 *  house-style allow-list. */
function manifest() {
	return `# GENERATED by tools/vendor-wgpu-ui.mjs - do not edit.
#
# The upstream manifest inherits from the wgpu-ui workspace; this one inherits
# from ours, so wgpu/winit/bytemuck resolve to this workspace's single pin and
# cannot drift. Lints are spelled out to keep wgpu-ui's stricter policy.
[package]
name = "wgpu-ui"
description = "Retained-mode GUI toolkit for wgpu: widget tree, layout, a batched 2D renderer, and headless offscreen testing"
version = "0.1.0"
edition.workspace = true
license.workspace = true
authors.workspace = true
publish = false

[dependencies]
wgpu.workspace = true
bytemuck.workspace = true
winit = { workspace = true, optional = true }

[features]
default = []
# Translate winit window events into \`wgpu_ui::Event\`s (see \`wgpu_ui::winit\`).
winit = ["dep:winit"]
# Upstream's \`secret\` (zeroize) and \`cosmic\` (cosmic-text) features are NOT
# offered here: each pulls a dependency this workspace does not vendor, and the
# editor uses neither. Their \`#[cfg]\`s in the mirrored src/ are declared to the
# cfg checker below instead, so the byte-identical source still lints clean.

[lints.rust]
unsafe_code = "deny"
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(feature, values("secret", "cosmic"))'] }

[lints.clippy]
all = { level = "warn", priority = -1 }
`;
}

function readme() {
	return `# wgpu-ui (vendored)

The first-party \`wgpu-ui\` toolkit, committed here so this repo builds without a
sibling checkout - a fresh clone, a CI runner or a release build has no other way
to get it (\`wgpu-ui\` has no public remote).

**This is the copy you edit.** It is what the path dep builds and what CI
compiles. The sibling checkout

    /media/ssd-projects/Rust/wgpu-ui      branch \`dev\`

is the shared working copy other projects consume, and carries the toolkit's own
test suite. Edit here, then push out and run those tests there:

    node tools/vendor-wgpu-ui.mjs           # push here -> sibling
    node tools/vendor-wgpu-ui.mjs --pull    # take work another agent did there
    node tools/vendor-wgpu-ui.mjs --check   # report anything unsynced (CI, when present)

The sync is **three-way**, not a blind copy: \`tools/wgpu-ui-sync.json\` holds the
hash of every file as of the last time the two sides agreed, so a push cannot
bury an edit made in the sibling by another project, and a pull cannot revert
one made here. Both sides changed = a conflict you resolve (or \`--force\`).

\`src/\`, \`assets/\` and \`rustfmt.toml\` are byte-identical on both sides.
\`Cargo.toml\` and this README are generated - the upstream manifest inherits from
the wgpu-ui workspace, and this copy has to inherit from ours instead. A toolkit
dependency or pin change is the one thing still made upstream, in the sibling's
manifest.

The vendored \`rustfmt.toml\` is load-bearing: rustfmt resolves the nearest
config to each file, so it keeps \`cargo fmt\` here reproducing upstream's style
(4 spaces) rather than this repo's (hard tabs, 120 cols), which would rewrite
every line and drown every sync in noise.

\`tests/\` is deliberately not mirrored: the toolkit's own suite runs in its own
repo, and its golden PNGs are several megabytes. The unit tests inside \`src/\`
do come along and run with \`cargo test\`.

The bundled \`assets/DejaVuSans.ttf\` is \`include_bytes!\`d by \`src/\` and ships
under its own license, kept beside it in \`assets/DejaVuSans-LICENSE.txt\`.
`;
}

// --- the mirrored file set, as (key, here, there) triples --------------------

/** Every file either side has, as `{key, here, there}` absolute-path pairs.
 *  `key` is what the baseline records: the path relative to the crate for the
 *  mirrored directories, the bare name for a workspace-root file. */
function pairs() {
	const out = [];
	for (const dir of MIRRORED) {
		const here = join(DEST, dir);
		const there = join(SRC_CRATE, dir);
		for (const f of [...new Set([...filesUnder(here), ...filesUnder(there)])].sort()) {
			out.push({ key: `${dir}/${f}`, here: join(here, f), there: join(there, f) });
		}
	}
	for (const f of MIRRORED_ROOT_FILES) {
		out.push({ key: f, here: join(DEST, f), there: join(SRC, f) });
	}
	return out;
}

function loadBaseline() {
	if (!existsSync(BASELINE)) return null;
	try {
		const j = JSON.parse(readFileSync(BASELINE, "utf8"));
		return j && typeof j.files === "object" ? j.files : null;
	} catch (e) {
		console.error(`vendor-wgpu-ui: cannot read ${relative(ROOT, BASELINE)}: ${e.message}`);
		process.exit(1);
	}
}

function writeBaseline(files) {
	const sorted = Object.fromEntries(Object.entries(files).sort(([a], [b]) => (a < b ? -1 : 1)));
	writeFileSync(
		BASELINE,
		`${JSON.stringify(
			{
				comment:
					"GENERATED by tools/vendor-wgpu-ui.mjs - the state crates/wgpu-ui and the sibling " +
					"checkout last agreed on. It is what makes the sync three-way: do not hand-edit, " +
					"and commit it with the sync it describes.",
				syncedAt: new Date().toISOString(),
				files: sorted,
			},
			null,
			"\t",
		)}\n`,
	);
}

/** One file's state: `sync` | `push` | `incoming` | `conflict`. With no
 *  baseline recorded (the first run, or a new file on both sides) a difference
 *  cannot be attributed to either side, so it is a conflict until forced. */
function classify(here, there, base) {
	if (here === there) return "sync";
	if (base !== undefined && there === base) return "push";
	if (base !== undefined && here === base) return "incoming";
	return "conflict";
}

function copyOrDelete(from, to) {
	if (existsSync(from)) {
		mkdirSync(dirname(to), { recursive: true });
		cpSync(from, to);
	} else if (existsSync(to)) {
		unlinkSync(to);
	}
}

// --- sibling present? ------------------------------------------------------

if (!existsSync(SRC_CRATE)) {
	if (CHECK) {
		// A fresh clone / CI runner has no sibling to compare against; the
		// committed copy is what builds, so only the generated files are checked.
		const bad = [
			["Cargo.toml", manifest()],
			["README.md", readme()],
		].filter(([f, want]) => !existsSync(join(DEST, f)) || readFileSync(join(DEST, f), "utf8") !== want);
		for (const [f] of bad) console.error(`vendor-wgpu-ui: GENERATED FILE EDITED BY HAND: ${f}`);
		if (bad.length) {
			console.error("vendor-wgpu-ui: re-run `node tools/vendor-wgpu-ui.mjs` to regenerate");
			process.exit(1);
		}
		console.log(`vendor-wgpu-ui: no sibling checkout at ${SRC} - skipping the sync check`);
		console.log("vendor-wgpu-ui: the committed crates/wgpu-ui is what builds");
		process.exit(0);
	}
	console.error(`vendor-wgpu-ui: no sibling checkout at ${SRC}`);
	console.error("vendor-wgpu-ui: set WGPU_UI_SRC to the wgpu-ui repo root and re-run");
	process.exit(1);
}

// --- pins ------------------------------------------------------------------

const ours = workspaceDeps(join(ROOT, "Cargo.toml"));
const theirs = workspaceDeps(join(SRC, "Cargo.toml"));
let pinFailed = false;
for (const name of SHARED) {
	const a = ours.get(name);
	const b = theirs.get(name);
	if (!a || !b) {
		console.error(`vendor-wgpu-ui: ${name}: missing pin (editor=${a ?? "?"}, wgpu-ui=${b ?? "?"})`);
		pinFailed = true;
	} else if (a !== b) {
		console.error(`vendor-wgpu-ui: ${name}: PIN MISMATCH editor="${a}" wgpu-ui="${b}"`);
		pinFailed = true;
	} else {
		console.log(`vendor-wgpu-ui: pin ${name} ${a} ok`);
	}
}
if (pinFailed) {
	console.error("vendor-wgpu-ui: align [workspace.dependencies] in both workspaces and re-run");
	process.exit(1);
}

// --- classify --------------------------------------------------------------

const base = loadBaseline();
if (!base && !CHECK && !FORCE) {
	// Nothing to attribute a difference to. Identical sides are fine (that IS
	// the agreed state); anything else needs a human to say which side wins.
	const differing = pairs().filter((p) => hashOf(p.here) !== hashOf(p.there));
	if (differing.length) {
		console.error(`vendor-wgpu-ui: no baseline at ${relative(ROOT, BASELINE)} and the two sides differ:`);
		for (const p of differing.slice(0, 20)) console.error(`vendor-wgpu-ui:   ${p.key}`);
		if (differing.length > 20) console.error(`vendor-wgpu-ui:   ... and ${differing.length - 20} more`);
		console.error("vendor-wgpu-ui: re-run with --force (push) or --pull --force to declare a winner");
		process.exit(1);
	}
}

const states = { sync: [], push: [], incoming: [], conflict: [] };
const hashes = new Map();
for (const p of pairs()) {
	const here = hashOf(p.here);
	const there = hashOf(p.there);
	hashes.set(p.key, { here, there });
	states[classify(here, there, base ? (p.key in base ? base[p.key] : null) : undefined)].push(p);
}

const label = (p) => {
	const { here, there } = hashes.get(p.key);
	if (here === null) return `${p.key} (gone here)`;
	if (there === null) return `${p.key} (gone upstream)`;
	return p.key;
};
for (const p of states.conflict) console.error(`vendor-wgpu-ui: CONFLICT (both sides changed): ${label(p)}`);
for (const p of states.incoming) console.error(`vendor-wgpu-ui: INCOMING (changed upstream): ${label(p)}`);
for (const p of states.push) console.log(`vendor-wgpu-ui: local change: ${label(p)}`);

// --- check -----------------------------------------------------------------

const generated = [
	["Cargo.toml", manifest()],
	["README.md", readme()],
];

if (CHECK) {
	let bad = states.conflict.length + states.incoming.length + states.push.length;
	for (const [f, want] of generated) {
		if (!existsSync(join(DEST, f)) || readFileSync(join(DEST, f), "utf8") !== want) {
			console.error(`vendor-wgpu-ui: GENERATED FILE EDITED BY HAND: ${f}`);
			bad++;
		}
	}
	if (!base) {
		console.error(`vendor-wgpu-ui: no baseline at ${relative(ROOT, BASELINE)} - run a sync to record one`);
		bad++;
	}
	if (bad) {
		console.error(`vendor-wgpu-ui: ${bad} item(s) unsynced`);
		console.error("vendor-wgpu-ui: push local edits with `node tools/vendor-wgpu-ui.mjs`,");
		console.error("vendor-wgpu-ui: take upstream ones with `node tools/vendor-wgpu-ui.mjs --pull`");
		process.exit(1);
	}
	console.log(`vendor-wgpu-ui: crates/wgpu-ui and ${SRC_CRATE} agree (${states.sync.length} files)`);
	process.exit(0);
}

// --- sync ------------------------------------------------------------------

// The direction's own work, plus what it may NOT do on its own: a push must not
// bury an upstream edit, a pull must not revert a local one.
const [take, blocked, blockedWord] = PULL
	? [states.incoming, states.push, "local change(s) not yet pushed"]
	: [states.push, states.incoming, "upstream change(s) not yet pulled"];

if (states.conflict.length && !FORCE) {
	console.error(`vendor-wgpu-ui: ${states.conflict.length} conflict(s) - both sides changed the same file`);
	console.error("vendor-wgpu-ui: reconcile them by hand, or re-run with --force to overwrite the other side");
	process.exit(1);
}
if (blocked.length && !FORCE) {
	console.error(`vendor-wgpu-ui: ${blocked.length} ${blockedWord}`);
	console.error(
		PULL
			? "vendor-wgpu-ui: push them first (`node tools/vendor-wgpu-ui.mjs`), or --force to discard them"
			: "vendor-wgpu-ui: pull them first (`--pull`), or --force to overwrite them",
	);
	process.exit(1);
}

const moving = FORCE ? [...take, ...states.conflict, ...blocked] : take;
for (const p of moving) {
	copyOrDelete(PULL ? p.there : p.here, PULL ? p.here : p.there);
	console.log(`vendor-wgpu-ui: ${PULL ? "pulled" : "pushed"} ${p.key}`);
}
// Prune directories the sync emptied, so a deleted module leaves no husk.
for (const dir of MIRRORED) {
	for (const root of [DEST, SRC_CRATE]) {
		if (existsSync(join(root, dir)) && filesUnder(join(root, dir)).length === 0) {
			rmSync(join(root, dir), { recursive: true, force: true });
		}
	}
}

for (const [f, want] of generated) {
	if (!existsSync(join(DEST, f)) || readFileSync(join(DEST, f), "utf8") !== want) {
		writeFileSync(join(DEST, f), want);
		console.log(`vendor-wgpu-ui: regenerated ${f}`);
	}
}

// Record what the two sides now agree on. A file left divergent on purpose (a
// pull that kept a local change) keeps its OLD baseline, so it still reports as
// a pending push next run.
const next = {};
let divergent = 0;
for (const p of pairs()) {
	const here = hashOf(p.here);
	const there = hashOf(p.there);
	if (here === there) {
		// Both gone drops out of the baseline entirely.
		if (here !== null) next[p.key] = here;
	} else {
		divergent++;
		if (base && p.key in base) next[p.key] = base[p.key];
	}
}
writeBaseline(next);

console.log(
	`vendor-wgpu-ui: ${PULL ? "pull" : "push"} done - ${moving.length} file(s) moved, ` +
		`${Object.keys(next).length} in sync${divergent ? `, ${divergent} still divergent` : ""}`,
);
