#!/usr/bin/env node
// Third-party license policy tool - no npm dependencies, plain Node.
//
//   node tools/licenses.mjs           regenerate THIRD-PARTY-LICENSES.md
//   node tools/licenses.mjs --check   verify policy + that the file is fresh
//                                     (exit 1 on violation/staleness; CI runs this)
//
// Policy: every external crate in the dependency graph must be available
// under a permissive license from the allow-list below. Copyleft (GPL/MPL/
// LGPL/…) is denied. Note: a pure-MIT-only policy is impossible here -
// winit is Apache-2.0 - so the list is "MIT-compatible permissive".
//
// The generated THIRD-PARTY-LICENSES.md bundles each crate's license text
// (read from the cargo registry sources) and is committed + shipped in the
// release zips, crediting the dependency authors.

import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUTPUT = join(ROOT, "THIRD-PARTY-LICENSES.md");
const CHECK = process.argv.includes("--check");

// Fonts are bundled too, and `cargo metadata` cannot see them: each is
// `include_bytes!`d straight into the binary, so it ships as surely as any
// crate does. Declared by hand because there is no manifest to read, and
// verified on every run - `--check` fails if a file named here is missing, so
// swapping a face without bringing its license along cannot pass CI.
//
// `text` is the license file kept beside the font. A first-party face has none:
// it is covered by the project's own LICENSE.
const FONTS = [
	{
		name: "Hack",
		version: "3.003",
		license: "MIT AND Bitstream-Vera",
		file: "app/assets/Hack-Regular.ttf",
		text: "app/assets/Hack-LICENSE.txt",
		source: "https://github.com/source-foundry/Hack",
		role: "the editor's monospace face - the console (`TextRole::Mono`)",
	},
	{
		name: "DejaVu Sans",
		version: "2.35",
		license: "Bitstream-Vera AND Public-Domain",
		file: "crates/wgpu-ui/assets/DejaVuSans.ttf",
		text: "crates/wgpu-ui/assets/DejaVuSans-LICENSE.txt",
		source: "https://dejavu-fonts.github.io/",
		role: "`wgpu-ui`'s built-in fallback face, so a host that supplies no font still draws text",
	},
	{
		name: "MAX Redesign Square",
		version: "2025-05-10",
		license: "MIT (first-party - see LICENSE)",
		file: "app/assets/MAX_Redesign_Square.ttf",
		text: null,
		source: null,
		role: "the chrome face - every menu, dialog and panel",
	},
];

// SPDX ids accepted (case-sensitive, as crates declare them).
const ALLOWED = new Set([
	"MIT",
	"Apache-2.0",
	"Apache-2.0 WITH LLVM-exception",
	"BSD-2-Clause",
	"BSD-3-Clause",
	"ISC",
	"Zlib",
	"BSL-1.0",
	"CC0-1.0",
	"Unicode-3.0",
	"Unicode-DFS-2016",
	"0BSD",
]);

/** A license expression passes if every AND-part has at least one allowed
 *  OR-alternative. Handles the flat `A OR B`, `A AND B`, `(A OR B) AND C`
 *  forms that appear on crates.io - not a full SPDX parser, and that's fine:
 *  anything it can't understand is a policy failure to look at by hand. */
function licenseAllowed(expr) {
	if (!expr) return false;
	// Split on AND at top level (parens only ever group OR-lists on crates.io).
	const andParts = expr.split(/\s+AND\s+/);
	return andParts.every((part) => {
		const alternatives = part.replaceAll("(", "").replaceAll(")", "").split(/\s+OR\s+|\//);
		return alternatives.some((alt) => ALLOWED.has(alt.trim()));
	});
}

const metadata = JSON.parse(
	execFileSync("cargo", ["metadata", "--format-version", "1", "--locked"], {
		cwd: ROOT,
		maxBuffer: 256 * 1024 * 1024,
	}),
);

const workspace = new Set(metadata.workspace_members);
const packages = metadata.packages
	.filter((p) => !workspace.has(p.id))
	.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));

// ---- policy check -----------------------------------------------------------
const violations = packages.filter((p) => !licenseAllowed(p.license));
if (violations.length > 0) {
	console.error("license policy violations (not in the permissive allow-list):");
	for (const p of violations) {
		console.error(`  ${p.name} ${p.version}: ${p.license ?? "<no license field>"}`);
	}
	process.exit(1);
}

// ---- the bundled fonts are really there -------------------------------------
const missing = FONTS.flatMap((f) => [f.file, f.text].filter(Boolean).filter((p) => !existsSync(join(ROOT, p))));
if (missing.length > 0) {
	console.error("declared in FONTS but not on disk:");
	for (const p of missing) console.error(`  ${p}`);
	process.exit(1);
}

// ---- attribution file -------------------------------------------------------
function licenseTexts(pkg) {
	const dir = dirname(pkg.manifest_path);
	let files;
	try {
		files = readdirSync(dir).filter((f) => /^(LICEN[CS]E|COPYING|NOTICE)/i.test(f));
	} catch {
		return [];
	}
	return files.sort().map((f) => {
		let text = "";
		try {
			// CRLF → LF: matches the repo's `* text=auto eol=lf` normalization,
			// so the committed file compares equal to a fresh regeneration.
			text = readFileSync(join(dir, f), "utf8").replaceAll("\r\n", "\n").trim();
		} catch {
			/* unreadable license file - listed without text below */
		}
		return { name: f, text };
	});
}

// Many crates ship byte-identical license texts (the Apache 2.0 body alone
// would repeat ~200×) - dedupe them into a shared appendix.
const uniqueTexts = []; // { label, text, files: Set<string> }
function textRef(file, text) {
	// Normalize trivial whitespace differences so identical bodies collapse.
	const key = text.replace(/\s+/g, " ").trim();
	let entry = uniqueTexts.find((t) => t.key === key);
	if (!entry) {
		entry = { key, label: `license-text-${uniqueTexts.length + 1}`, text, files: new Set() };
		uniqueTexts.push(entry);
	}
	entry.files.add(file);
	return entry.label;
}

let body = "";
for (const p of packages) {
	body += `\n## ${p.name} ${p.version}\n\n`;
	body += `- License: ${p.license}\n`;
	if (p.repository) body += `- Source: ${p.repository}\n`;
	const texts = licenseTexts(p);
	if (texts.length === 0) {
		body += `- License text: none shipped in the crate package; the terms are the SPDX license(s) named above.\n`;
		continue;
	}
	for (const { name, text } of texts) {
		body += `- ${name}: see [${textRef(name, text)}](#${textRef(name, text)})\n`;
	}
}

for (const f of FONTS) {
	body += `\n## ${f.name} ${f.version}\n\n`;
	body += `- License: ${f.license}\n`;
	if (f.source) body += `- Source: ${f.source}\n`;
	body += `- Bundled as: \`${f.file}\`\n`;
	body += `- Used for: ${f.role}\n`;
	if (f.text) {
		const name = f.text.split("/").pop();
		const text = readFileSync(join(ROOT, f.text), "utf8").replaceAll("\r\n", "\n").trim();
		body += `- ${name}: see [${textRef(name, text)}](#${textRef(name, text)})\n`;
	} else {
		body += `- License text: this project's own LICENSE - the face was drawn for it.\n`;
	}
}

let out = `# Third-party licenses

The M.A.X. Map Editor bundles the following Rust crates and fonts. Each is used
under the license named beside it; the full texts are in the appendix (identical
texts are listed once and shared). This file is generated by
\`tools/licenses.mjs\` from Cargo.lock and the font list in that script - do not
edit by hand.

| Crate | Version | License |
|---|---|---|
`;
for (const p of packages) {
	out += `| ${p.name} | ${p.version} | ${p.license} |\n`;
}
out += `
The fonts are compiled into the binary rather than resolved from the system, so
the editor draws the same text everywhere and needs nothing installed:

| Font | Version | License |
|---|---|---|
`;
for (const f of FONTS) {
	out += `| ${f.name} | ${f.version} | ${f.license} |\n`;
}
out += "\n---\n";
out += body;
out += "\n---\n\n# Appendix: license texts\n";
for (const t of uniqueTexts) {
	out += `\n## ${t.label}\n\n\`\`\`\n${t.text}\n\`\`\`\n`;
}

if (CHECK) {
	const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8") : "";
	if (current !== out) {
		console.error("THIRD-PARTY-LICENSES.md is stale - run: node tools/licenses.mjs");
		process.exit(1);
	}
	console.log(`license policy OK (${packages.length} external crates, file fresh)`);
} else {
	writeFileSync(OUTPUT, out);
	console.log(`wrote THIRD-PARTY-LICENSES.md (${packages.length} external crates)`);
}
