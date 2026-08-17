#!/usr/bin/env node
// Build the bundled HTML user manual (Help ▸ User Manual) from MANUAL.md.
//
//   node tools/build-manual.mjs            # assemble HTML from MANUAL.md + existing screenshots
//   node tools/build-manual.mjs --shots    # also (re)capture the screenshots headless, then assemble
//
// Pure Node, no dependencies (hand-rolled Markdown → HTML for the subset
// MANUAL.md uses: headings, paragraphs, ordered/unordered lists, GitHub-style
// tables, fenced + inline code, bold/italic, links, horizontal rules). The
// result is a single self-contained `resources/manual/index.html` (CSS inline);
// screenshots are the only external files, pulled from `resources/manual/img/`.
//
// Screenshots are injected after the headings named in SHOTS below, but only
// when the PNG already exists - so MANUAL.md stays image-free and the manual
// renders cleanly before any screenshots are captured. Generate the PNGs with
// the editor's headless `screenshot` command (see tools/manual-shots.script),
// then re-run this tool to fold them in.

import { readFileSync, readdirSync, writeFileSync, existsSync, mkdirSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SRC = join(ROOT, 'MANUAL.md');
const OUT_DIR = join(ROOT, 'resources', 'manual');
const IMG_DIR = join(OUT_DIR, 'img');
const OUT = join(OUT_DIR, 'index.html');

// Screenshots, each folded into the HTML after the named heading and - with
// `--shots` - captured headless by driving `cmds` then a `screenshot`. Each
// scene runs in its own process, so an open modal in one can't bleed into the
// next (there's no close-modal command).
//
// `after` matches the heading text with its leading "N. " stripped, so
// renumbering a section never silently orphans its screenshot (it did once).
// `needs` gates a scene on a repo file that may be absent (e.g. the gitignored
// original WRLs and save fixtures); `needsGame` gates it on the user's MaxPath
// and lends that scene - and only that scene - the user's `[Paths]`, so game
// sprites resolve. A gated scene is skipped with a warning rather than failing,
// so the manual still builds on a machine with no M.A.X. install.
const SHOTS = [
	{
		after: 'Getting started',
		file: 'workspace.png',
		caption: 'The editor workspace: menu bar, map canvas, docked panels, and status bar.',
		cmds: ['window minimap on', 'window tiles on', 'window toolbox on', 'window palette off', 'window units off', 'window wrlpalette off', 'fit'],
	},
	{
		after: 'Documents',
		file: 'importwrl.png',
		caption: 'Import WRL: match a standard-tile map onto existing tilesets.',
		cmds: ['import-wrl testdata/originals/DESERT_1.WRL'],
		needs: 'testdata/originals/DESERT_1.WRL',
	},
	{
		after: 'The workspace',
		file: 'panels.png',
		caption: 'Dockable panels - Minimap, Tile Explorer, and the Color Palette.',
		cmds: ['window minimap on', 'window tiles on', 'window toolbox on', 'window palette on', 'fit'],
	},
	{
		after: 'Editing',
		file: 'editing.png',
		caption: 'Editing terrain - a marked selection, the Tile Explorer, and the toolbox.',
		cmds: ['window minimap on', 'window tiles on', 'window toolbox on', 'window palette off', 'fit', 'select-rect 38 40 74 72'],
	},
	{
		after: 'The palette',
		file: 'palette.png',
		caption: 'The Color Palette panel and its palette manager.',
		cmds: ['window minimap off', 'window tiles off', 'window toolbox off', 'window palette on'],
	},
	{
		after: 'Unit previews',
		file: 'units.png',
		caption: 'The Units panel - real game sprites stamped on the map, recolored to their team.',
		needsGame: true,
		cmds: [
			'new 40 28 GREEN 7', 'generate land seed=1996 obstructions=3 decorations=5 shore=sweep',
			'window templates off', 'window minimap off', 'window tiles off', 'window toolbox off',
			'window palette off', 'window unitprops off', 'window units on',
			'unit-team red', 'unit-place TANK 12 12', 'unit-place SCOUT 14 11', 'unit-place ENGINEER 13 14',
			'unit-team blue', 'unit-place TANK 18 13', 'unit-place SCANNER 19 11',
			'units on', 'zoom-to 1.0', 'pan-to 16 13',
		],
	},
	{
		after: 'Save files',
		file: 'saveeditor.png',
		caption: 'The save editor: a V71 save opened, with the Save Toolbox and the Unit Properties inspector.',
		needs: 'testdata/saves/save11-green3-50x50.dta',
		needsGame: true,
		cmds: [
			'open-save testdata/saves/save11-green3-50x50.dta', 'mode save',
			'window savetools on', 'window unitprops on', 'window units off',
			'window templates off', 'window minimap off', 'window tiles off', 'window palette off',
			'window toolbox off', 'tool obj-select', 'object-select 29 39',
			'resources on', 'zoom-to 1.0', 'pan-to 29 39',
		],
	},
	{
		after: 'The console',
		file: 'console.png',
		caption: 'The in-app console - every editor action is a typeable command.',
		cmds: ['console on'],
	},
];

/// The user's `[Paths]` section, read (never written) from their config so the
/// game-data scenes can find MAX.RES and the saves. Returns the raw lines plus
/// the two directories the scene gating needs.
function userPaths() {
	const file = join(ROOT, 'resources/user/config/mme.ini');
	if (!existsSync(file)) return null;
	const section = readFileSync(file, 'utf8').split(/\r?\n/);
	const start = section.findIndex((l) => l.trim().toLowerCase() === '[paths]');
	if (start < 0) return null;
	const lines = [];
	for (const line of section.slice(start + 1)) {
		if (line.trim().startsWith('[')) break;
		if (line.trim()) lines.push(line.trim());
	}
	const value = (key) => {
		const hit = lines.find((l) => l.toLowerCase().startsWith(`${key.toLowerCase()}=`));
		const v = hit?.slice(hit.indexOf('=') + 1).trim();
		return v || null;
	};
	return { lines, maxPath: value('MaxPath'), maxPortPath: value('MaxPortPath') };
}

/// Capture every scene headless (build once, one process per scene), writing
/// resources/manual/img/<file>.png. Needs a GPU or software adapter (lavapipe).
function runShots() {
	console.log('building the editor (cargo build)…');
	if (spawnSync('cargo', ['build', '--quiet'], { cwd: ROOT, stdio: 'inherit' }).status !== 0) {
		console.error('cargo build failed');
		process.exit(1);
	}
	const bin = join(ROOT, 'target', 'debug', 'max-map-editor');
	const tmp = join(ROOT, 'temp');
	mkdirSync(tmp, { recursive: true });
	mkdirSync(IMG_DIR, { recursive: true });
	const scenePath = join(tmp, '.manual-scene.script');
	// Capture against a throwaway COPY of the shipped config, so screenshots show
	// the shipped defaults (e.g. explorer thumbnail sizes) rather than the
	// hardcoded headless ones - and the binary's save-on-exit hits the copy, not
	// the real file. Re-copied per scene so no scene's exit-state drifts the next.
	const shipped = join(ROOT, 'resources/config/mme.ini');
	const settings = existsSync(shipped) ? join(tmp, '.manual-settings.ini') : null;
	const paths = userPaths();
	for (const shot of SHOTS) {
		if (shot.needs && !existsSync(join(ROOT, shot.needs))) {
			console.warn(`  skip ${shot.file} - needs ${shot.needs} (absent)`);
			continue;
		}
		if (shot.needsGame && !paths?.maxPath) {
			console.warn(`  skip ${shot.file} - needs a M.A.X. install ([Paths] MaxPath is unset)`);
			continue;
		}
		const lines = [...(shot.cmds ?? []), `screenshot resources/manual/img/${shot.file}`];
		writeFileSync(scenePath, lines.join('\n') + '\n');
		const args = ['--headless', '--script', scenePath];
		if (settings) {
			// A throwaway copy of the SHIPPED config, plus - only for the scenes
			// that asked - the user's [Paths] so MAX.RES and the saves resolve.
			// The user's own file is read, never written: the binary's
			// save-on-exit lands on this copy.
			const extra = (shot.needsGame || shot.needsSave) && paths ? `\n[Paths]\n${paths.lines.join('\n')}\n` : '';
			writeFileSync(settings, readFileSync(shipped) + extra);
			args.unshift('--settings', settings);
		}
		const r = spawnSync(bin, args, { cwd: ROOT, stdio: ['ignore', 'ignore', 'inherit'] });
		if (r.status !== 0) console.warn(`  ${shot.file}: capture exited with ${r.status}`);
	}
	rmSync(scenePath, { force: true });
	if (settings) rmSync(settings, { force: true });
}

// ---- inline markdown -------------------------------------------------------

function escapeHtml(s) {
	return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/// Text-node escaping plus the quotes - for values that land *inside* an
/// attribute, where a bare `"` or `'` would close it early.
function escapeAttr(s) {
	return escapeHtml(s).replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

function inline(text) {
	let s = escapeHtml(text);
	// Protect code spans from the other inline rules.
	const codes = [];
	s = s.replace(/`([^`]+)`/g, (_, c) => ` ${codes.push(c) - 1} `);
	s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
	s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
	s = s.replace(/(^|[^*])\*([^*\s][^*]*?)\*/g, '$1<em>$2</em>');
	s = s.replace(/(^|[^_\w])_([^_\s][^_]*?)_/g, '$1<em>$2</em>');
	s = s.replace(/ (\d+) /g, (_, i) => `<code>${codes[+i]}</code>`);
	return s;
}

function slug(text) {
	return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
}

// ---- block markdown --------------------------------------------------------

/// Shots that found their heading this run - so an `after` that matches nothing
/// (a renamed section) is reported instead of vanishing quietly.
const placed = new Set();

function figureFor(headingText) {
	// Match on the title alone: "7. Save files (experimental)" -> "save files…".
	const title = headingText.replace(/^\s*\d+\.\s*/, '').toLowerCase();
	const shot = SHOTS.find((s) => title.startsWith(s.after.toLowerCase()));
	if (!shot) return '';
	placed.add(shot.file);
	if (!existsSync(join(IMG_DIR, shot.file))) {
		console.warn(`  (skipping screenshot img/${shot.file} - not captured yet)`);
		return '';
	}
	return `<figure><img src="img/${shot.file}" alt="${escapeAttr(shot.caption)}" loading="lazy">` +
		`<figcaption>${escapeHtml(shot.caption)}</figcaption></figure>\n`;
}

function renderTable(rows) {
	// rows[0] = header, rows[1] = separator (discarded), rest = body.
	const cells = (line) => line.replace(/^\||\|$/g, '').split('|').map((c) => c.trim());
	const head = cells(rows[0]);
	const body = rows.slice(2).map(cells);
	let out = '<table>\n<thead><tr>';
	out += head.map((c) => `<th>${inline(c)}</th>`).join('');
	out += '</tr></thead>\n<tbody>\n';
	for (const r of body) {
		out += '<tr>' + head.map((_, i) => `<td>${inline(r[i] ?? '')}</td>`).join('') + '</tr>\n';
	}
	out += '</tbody></table>\n';
	return out;
}

const isTableSep = (l) => /^\|?[\s:|-]+\|?$/.test(l) && l.includes('-');
const indentOf = (l) => (l.match(/^[\t ]*/)[0] || '').replace(/\t/g, '  ').length;

function render(md) {
	const lines = md.replace(/\r\n/g, '\n').split('\n');
	let html = '';
	let para = [];
	const toc = [];

	const flushPara = () => {
		if (para.length) {
			html += `<p>${inline(para.join(' '))}</p>\n`;
			para = [];
		}
	};

	// A list is parsed as a small recursive block by indentation.
	function parseList(i) {
		const baseIndent = indentOf(lines[i]);
		const ordered = /^\s*\d+\.\s/.test(lines[i]);
		const tag = ordered ? 'ol' : 'ul';
		let out = `<${tag}>\n`;
		while (i < lines.length) {
			const line = lines[i];
			if (line.trim() === '') {
				// A blank line ends the list unless the next line continues it (more indented / another item).
				const next = lines[i + 1] ?? '';
				if (next.trim() === '' || (indentOf(next) < baseIndent + 1 && !/^\s*([-*]|\d+\.)\s/.test(next))) break;
				i++;
				continue;
			}
			const m = line.match(/^(\s*)([-*]|\d+\.)\s+(.*)$/);
			if (!m) break;
			const indent = indentOf(line);
			if (indent < baseIndent) break;
			if (indent > baseIndent) {
				// Nested list: recurse and graft onto the previous <li>.
				const [nested, ni] = parseList(i);
				out = out.replace(/<\/li>\n$/, `${nested}</li>\n`);
				i = ni;
				continue;
			}
			let text = m[3];
			i++;
			// Continuation lines (indented, no marker) belong to this item.
			while (i < lines.length && lines[i].trim() !== '' && !/^\s*([-*]|\d+\.)\s/.test(lines[i]) && indentOf(lines[i]) > baseIndent) {
				text += ' ' + lines[i].trim();
				i++;
			}
			out += `<li>${inline(text)}</li>\n`;
		}
		out += `</${tag}>\n`;
		return [out, i];
	}

	for (let i = 0; i < lines.length; ) {
		const line = lines[i];

		// Fenced code.
		if (/^```/.test(line)) {
			flushPara();
			i++;
			let code = '';
			while (i < lines.length && !/^```/.test(lines[i])) code += lines[i++] + '\n';
			i++; // closing fence
			html += `<pre><code>${escapeHtml(code.replace(/\n$/, ''))}</code></pre>\n`;
			continue;
		}

		// Heading.
		const h = line.match(/^(#{1,6})\s+(.*)$/);
		if (h) {
			flushPara();
			const level = h[1].length;
			const text = h[2].trim();
			const id = slug(text);
			if (level === 2) toc.push({ id, text });
			html += `<h${level} id="${id}">${inline(text)}</h${level}>\n`;
			html += figureFor(text);
			i++;
			continue;
		}

		// Horizontal rule.
		if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
			flushPara();
			html += '<hr>\n';
			i++;
			continue;
		}

		// Table: a pipe row followed by a separator row.
		if (/^\s*\|/.test(line) && i + 1 < lines.length && isTableSep(lines[i + 1])) {
			flushPara();
			const rows = [];
			while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(lines[i++].trim());
			html += renderTable(rows);
			continue;
		}

		// List.
		if (/^\s*([-*]|\d+\.)\s+/.test(line)) {
			flushPara();
			const [out, ni] = parseList(i);
			html += out;
			i = ni;
			continue;
		}

		// Blank line ends a paragraph.
		if (line.trim() === '') {
			flushPara();
			i++;
			continue;
		}

		para.push(line.trim());
		i++;
	}
	flushPara();
	return { html, toc };
}

// ---- page template ---------------------------------------------------------

const CSS = `
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body {
	margin: 0; background: #16181c; color: #d6d9de;
	font: 16px/1.65 ui-sans-serif, system-ui, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}
.wrap { max-width: 860px; margin: 0 auto; padding: 40px 24px 96px; }
h1, h2, h3, h4 { color: #f2f4f7; line-height: 1.25; font-weight: 650; }
h1 { font-size: 2.1rem; margin: 0 0 .2em; }
h2 { font-size: 1.5rem; margin: 2.2em 0 .6em; padding-top: .4em; border-top: 1px solid #2b2f36; }
h3 { font-size: 1.2rem; margin: 1.6em 0 .4em; color: #cdd2da; }
h4 { font-size: 1.02rem; margin: 1.2em 0 .3em; color: #aeb4be; }
a { color: #6fb3ff; text-decoration: none; }
a:hover { text-decoration: underline; }
p { margin: .7em 0; }
ul, ol { margin: .6em 0; padding-left: 1.6em; }
li { margin: .25em 0; }
li > ul, li > ol { margin: .2em 0; }
code { background: #23272e; border: 1px solid #2f343c; border-radius: 4px; padding: .08em .38em; font: .88em ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace; color: #e7c98a; }
pre { background: #1b1e23; border: 1px solid #2f343c; border-radius: 8px; padding: 14px 16px; overflow-x: auto; }
pre code { background: none; border: 0; padding: 0; color: #cdd2da; }
hr { border: 0; border-top: 1px solid #2b2f36; margin: 2em 0; }
table { border-collapse: collapse; width: 100%; margin: 1em 0; font-size: .94em; }
th, td { border: 1px solid #2f343c; padding: 7px 11px; text-align: left; vertical-align: top; }
thead th { background: #21252b; color: #f2f4f7; }
tbody tr:nth-child(even) { background: #1b1e23; }
figure { margin: 1.4em 0; }
figure img { display: block; width: 100%; height: auto; border: 1px solid #2f343c; border-radius: 8px; background: #0e0f12; }
figcaption { color: #8a909b; font-size: .88em; margin-top: .5em; text-align: center; }
nav.toc { background: #1b1e23; border: 1px solid #2b2f36; border-radius: 8px; padding: 14px 20px; margin: 1.6em 0 2.4em; }
nav.toc strong { display: block; color: #f2f4f7; margin-bottom: .4em; }
nav.toc ol { columns: 2; column-gap: 28px; margin: 0; padding-left: 1.3em; }
.tagline { color: #8a909b; }
footer { margin-top: 4em; padding-top: 1.2em; border-top: 1px solid #2b2f36; color: #6c727c; font-size: .88em; }
`;

function page(title, tocHtml, bodyHtml) {
	return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)}</title>
<style>${CSS}</style>
</head>
<body>
<main class="wrap">
${tocHtml}
${bodyHtml}
<footer>Generated from MANUAL.md · <a href="https://suns-echoes.github.io/max-map-editor/" target="_blank" rel="noopener">Website</a> · <a href="https://github.com/suns-echoes/max-map-editor" target="_blank" rel="noopener">GitHub</a></footer>
</main>
</body>
</html>
`;
}

// ---- main ------------------------------------------------------------------

// `--shots` (re)captures the screenshots headless before assembling the HTML.
if (process.argv.includes('--shots')) runShots();

const md = readFileSync(SRC, 'utf8');
const { html, toc } = render(md);
const tocHtml =
	'<nav class="toc"><strong>Contents</strong><ol>' +
	toc.map((t) => `<li><a href="#${t.id}">${inline(t.text)}</a></li>`).join('') +
	'</ol></nav>';

mkdirSync(IMG_DIR, { recursive: true });
writeFileSync(OUT, page('M.A.X. Map Editor - Manual', tocHtml, html));
for (const shot of SHOTS) {
	if (!placed.has(shot.file)) console.warn(`  WARNING: no heading starts with "${shot.after}" - img/${shot.file} is orphaned`);
}
const shots = SHOTS.filter((s) => placed.has(s.file) && existsSync(join(IMG_DIR, s.file))).length;
console.log(`wrote ${OUT}  (${toc.length} sections, ${shots}/${SHOTS.length} screenshots placed)`);
