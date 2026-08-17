// Scenario suite for tools/vendor-wgpu-ui.mjs: drives its three-way sync logic
// against a throwaway pair of trees under temp/ (an "editor" holding the
// vendored crate, and a stand-in sibling checkout), asserting that a push never
// buries an upstream edit, a pull never reverts a local one, and a file both
// sides changed reports as a conflict until forced.
//
// Run: node tools/vendor-wgpu-ui.test.mjs   (no dependencies, no GPU, ~1 s)
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PLAY = join(REPO, "temp/synctest");
const HERE = join(PLAY, "editor");
const THERE = join(PLAY, "sibling");
const TOOL = join(HERE, "tools/vendor-wgpu-ui.mjs");

const PINS = `[workspace.dependencies]
wgpu = { version = "=30.0.0" }
bytemuck = { version = "=1.25.1" }
winit = { version = "=0.30.13" }
`;

function reset() {
	rmSync(PLAY, { recursive: true, force: true });
	for (const [root, crate] of [
		[HERE, join(HERE, "crates/wgpu-ui")],
		[THERE, join(THERE, "crates/wgpu-ui")],
	]) {
		mkdirSync(join(crate, "src"), { recursive: true });
		mkdirSync(join(crate, "assets"), { recursive: true });
		writeFileSync(join(root, "Cargo.toml"), PINS);
		// rustfmt.toml lives at the sibling's WORKSPACE root but inside the
		// vendored crate here - the one asymmetric path in the mirrored set.
		writeFileSync(join(root === HERE ? crate : root, "rustfmt.toml"), "hard_tabs = false\n");
		writeFileSync(join(crate, "src/lib.rs"), "// v0\n");
		writeFileSync(join(crate, "src/widget.rs"), "// widget v0\n");
		writeFileSync(join(crate, "assets/font.ttf"), "TTF0");
	}
	mkdirSync(join(HERE, "tools"), { recursive: true });
	cpSync(join(REPO, "tools/vendor-wgpu-ui.mjs"), TOOL);
}

/** Runs the tool; returns {code, out}. */
function run(...args) {
	try {
		const out = execFileSync("node", [TOOL, ...args], {
			env: { ...process.env, WGPU_UI_SRC: THERE },
			encoding: "utf8",
			stdio: ["ignore", "pipe", "pipe"],
		});
		return { code: 0, out };
	} catch (e) {
		return { code: e.status ?? 1, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
	}
}

let failures = 0;
function check(name, cond, detail = "") {
	console.log(`${cond ? "  ok  " : "FAIL  "}${name}${cond ? "" : ` -- ${detail}`}`);
	if (!cond) failures++;
}
const read = (root, p) => (existsSync(join(root, "crates/wgpu-ui", p)) ? readFileSync(join(root, "crates/wgpu-ui", p), "utf8") : null);

// --- 1. first sync of two identical trees records a baseline ---------------
reset();
let r = run();
check("initial push succeeds", r.code === 0, r.out);
check("baseline written", existsSync(join(HERE, "tools/wgpu-ui-sync.json")), "no wgpu-ui-sync.json");
check("check is clean", run("--check").code === 0);

// --- 2. a local edit pushes out --------------------------------------------
writeFileSync(join(HERE, "crates/wgpu-ui/src/lib.rs"), "// v1 local\n");
r = run("--check");
check("check fails on an unpushed local edit", r.code === 1, r.out);
check("...and names it as a local change", r.out.includes("local change: src/lib.rs"), r.out);
r = run();
check("push succeeds", r.code === 0, r.out);
check("the sibling got it", read(THERE, "src/lib.rs") === "// v1 local\n", read(THERE, "src/lib.rs"));

// --- 3. an upstream edit is NOT buried by a push ----------------------------
writeFileSync(join(THERE, "crates/wgpu-ui/src/widget.rs"), "// widget v1 upstream\n");
r = run();
check("push refuses while an upstream edit is unpulled", r.code === 1, r.out);
check("...and names it INCOMING", r.out.includes("INCOMING (changed upstream): src/widget.rs"), r.out);
check("the upstream edit survived", read(THERE, "src/widget.rs") === "// widget v1 upstream\n");
r = run("--pull");
check("pull succeeds", r.code === 0, r.out);
check("the edit came in", read(HERE, "src/widget.rs") === "// widget v1 upstream\n", read(HERE, "src/widget.rs"));

// --- 4. both sides edit the same file = conflict ----------------------------
writeFileSync(join(HERE, "crates/wgpu-ui/src/lib.rs"), "// v2 mine\n");
writeFileSync(join(THERE, "crates/wgpu-ui/src/lib.rs"), "// v2 theirs\n");
r = run();
check("push refuses on a conflict", r.code === 1, r.out);
check("...and names it CONFLICT", r.out.includes("CONFLICT (both sides changed): src/lib.rs"), r.out);
r = run("--pull");
check("pull refuses on the same conflict", r.code === 1, r.out);
check("neither side was touched", read(HERE, "src/lib.rs") === "// v2 mine\n" && read(THERE, "src/lib.rs") === "// v2 theirs\n");
r = run("--force");
check("--force resolves it in the push direction", r.code === 0, r.out);
check("the sibling took mine", read(THERE, "src/lib.rs") === "// v2 mine\n", read(THERE, "src/lib.rs"));

// --- 5. a pull keeps an unpushed local change (does not revert it) ----------
writeFileSync(join(HERE, "crates/wgpu-ui/src/lib.rs"), "// v3 mine only\n");
writeFileSync(join(THERE, "crates/wgpu-ui/src/widget.rs"), "// widget v2 upstream\n");
r = run("--pull");
check("pull refuses while a local edit is unpushed", r.code === 1, r.out);
check("...and says so", r.out.includes("local change(s) not yet pushed"), r.out);
r = run("--pull", "--force");
check("--pull --force takes upstream", r.code === 0, r.out);
check("the pulled file landed", read(HERE, "src/widget.rs") === "// widget v2 upstream\n");
check("and the forced pull reverted my unpushed edit", read(HERE, "src/lib.rs") === "// v2 mine\n", read(HERE, "src/lib.rs"));

// --- 6. deletions propagate -------------------------------------------------
run();
rmSync(join(HERE, "crates/wgpu-ui/src/widget.rs"));
r = run();
check("a local delete pushes out", r.code === 0, r.out);
check("the file is gone upstream too", read(THERE, "src/widget.rs") === null);

// --- 7. a new file on one side only ----------------------------------------
writeFileSync(join(THERE, "crates/wgpu-ui/src/menu.rs"), "// new upstream module\n");
r = run("--check");
check("check flags a new upstream file", r.code === 1, r.out);
r = run("--pull");
check("pull takes the new file", r.code === 0 && read(HERE, "src/menu.rs") === "// new upstream module\n", r.out);

// --- 8. a hand-edited generated file is caught ------------------------------
writeFileSync(join(HERE, "crates/wgpu-ui/Cargo.toml"), "# hand edited\n");
r = run("--check");
check("check catches a hand-edited generated manifest", r.code === 1 && r.out.includes("GENERATED FILE EDITED BY HAND"), r.out);
r = run();
check("a push regenerates it", r.code === 0 && read(HERE, "Cargo.toml").startsWith("# GENERATED"), r.out);
check("final check is clean", run("--check").code === 0);

// Leave the trees behind on failure - they are the evidence.
if (!failures) rmSync(PLAY, { recursive: true, force: true });
console.log(failures ? `\n${failures} FAILURE(S) - trees left in ${PLAY}` : "\nall scenarios passed");
process.exit(failures ? 1 : 0);
