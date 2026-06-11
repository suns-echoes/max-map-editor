//! Tile Editing Toolbox dockable (design: features.drawio
//! "Dockables"): command-bound button groups, no logic of its own — every
//! live button runs a command line (the menu's pattern), unbuilt tools are
//! dim Todo placeholders echoing their ticket. The left edge previews the
//! active tile **with its transform** (the flip/rotate feedback).

use crate::picker::{self, TileQuad};
use crate::state::{EditorState, Tool};
use crate::theme;
use crate::ui::{Hot, Rect, SteelMap, UiQuads};

const PAD: f32 = 6.0;
const PREVIEW_W: f32 = 118.0;
const PREVIEW_PX: f32 = 48.0;
const BTN_W: f32 = 66.0;
const BTN_H: f32 = 18.0;
const GAP: f32 = 3.0; // between buttons within a group
const GROUP_GAP: f32 = 14.0; // between group blocks on a row
const ROW_GAP: f32 = 8.0; // between wrapped rows
const GROUP_LABEL_H: f32 = 14.0;

pub enum Act {
	/// A command line (validated against the parser by a test).
	Run(&'static str),
	/// Not built yet — echoes the ticket.
	Todo(&'static str),
}

pub struct Button {
	pub label: &'static str,
	pub act: Act,
	/// Optional swatch fill (pass-type buttons use the pass colors).
	pub fill: Option<[f32; 4]>,
}

pub struct Group {
	pub label: &'static str,
	pub cols: usize,
	pub buttons: &'static [Button],
}

/// A plain command button.
const fn b(label: &'static str, act: Act) -> Button {
	Button { label, act, fill: None }
}
/// A colored swatch button (pass types).
const fn sw(label: &'static str, act: Act, fill: [f32; 4]) -> Button {
	Button { label, act, fill: Some(fill) }
}

pub const GROUPS: &[Group] = &[
	Group {
		label: "draw",
		cols: 1,
		buttons: &[
			b("pencil", Act::Run("tool pencil")),
			b("pick", Act::Run("tool picker")),
			b("erase", Act::Run("tool eraser")),
			b("flood", Act::Run("tool fill")),
			b("random", Act::Run("randomize toggle")),
		],
	},
	Group {
		label: "layer",
		cols: 1,
		buttons: &[b("water", Act::Run("layer water")), b("ground", Act::Run("layer ground"))],
	},
	Group {
		label: "transform",
		cols: 2,
		buttons: &[
			b("flip h", Act::Run("transform flip-h")),
			b("flip v", Act::Run("transform flip-v")),
			b("rot cw", Act::Run("transform cw")),
			b("rot ccw", Act::Run("transform ccw")),
		],
	},
	Group {
		label: "auto paint",
		cols: 1,
		buttons: &[
			b("land", Act::Todo("TOOL-3")),
			b("shore", Act::Run("shore")),
			b("shore alt", Act::Run("shore alt")),
			b("shore fix", Act::Run("shore fix")),
			b("water", Act::Todo("TOOL-3")),
		],
	},
	Group {
		label: "pass type",
		cols: 2,
		buttons: &[
			sw("land", Act::Run("pass-pick 0"), crate::state::PASS_COLORS[0]),
			sw("water", Act::Run("pass-pick 1"), crate::state::PASS_COLORS[1]),
			sw("shore", Act::Run("pass-pick 2"), crate::state::PASS_COLORS[2]),
			sw("block", Act::Run("pass-pick 3"), crate::state::PASS_COLORS[3]),
		],
	},
	Group {
		label: "selection",
		cols: 2,
		buttons: &[
			b("select", Act::Run("tool select")),
			b("rect", Act::Run("tool select-rect")),
			b("clear", Act::Run("select clear")),
			b("same", Act::Run("select similar")),
		],
	},
	Group {
		label: "advanced",
		cols: 2,
		buttons: &[
			b("replace", Act::Todo("TOOL-12")),
			b("template", Act::Todo("TOOL-5")),
			b("clone", Act::Todo("TOOL-7")),
			b("capture", Act::Todo("TOOL-7")),
			b("import", Act::Todo("IO-4")),
			b("export", Act::Todo("IO-4")),
		],
	},
];

/// The pixel size of group `g`'s button block (the label sits above it).
fn group_size(g: usize) -> (f32, f32) {
	let group = &GROUPS[g];
	let rows = group.buttons.len().div_ceil(group.cols);
	let w = group.cols as f32 * BTN_W + (group.cols as f32 - 1.0) * GAP;
	let h = GROUP_LABEL_H + rows as f32 * BTN_H + rows.saturating_sub(1) as f32 * GAP;
	(w, h)
}

/// The flowed toolbox layout: the tile-preview box, then each group's
/// top-left block origin. Blocks flow left-to-right and **wrap to a new line
/// when the next one won't fit** — so a wide bottom dock keeps everything on
/// one row, while a narrow dock stacks the sections vertically. Drawing and
/// hit-testing share this, so the keys you click are the keys drawn.
pub struct Layout {
	/// The 48-px active-tile preview box.
	pub preview: Rect,
	/// Each group's block origin (top-left of its label); size = [`group_size`].
	pub groups: Vec<Rect>,
	/// Total flowed content height (scroll-independent) — drives the scrollbar.
	pub content_h: f32,
}

/// The flowed layout, shifted up by `scroll`. The right edge reserves the
/// scrollbar gutter so blocks never sit under the bar.
pub fn layout(body: Rect, scroll: f32) -> Layout {
	let left = body.x + PAD;
	let right = body.x + body.w - PAD - crate::ui::SCROLLBAR_W;
	let mut x = left;
	let top = body.y + PAD - scroll;
	let mut y = top;
	// The tile preview is the first block in the flow.
	let preview = Rect::new(x, y + GROUP_LABEL_H, PREVIEW_PX, PREVIEW_PX);
	let mut row_h = GROUP_LABEL_H + PREVIEW_PX;
	x += PREVIEW_W + GROUP_GAP;

	let mut groups = Vec::with_capacity(GROUPS.len());
	for g in 0..GROUPS.len() {
		let (gw, gh) = group_size(g);
		// Wrap when this block overflows the row (but never on an empty row,
		// so an over-wide group still gets placed rather than looping).
		if x + gw > right && x > left {
			x = left;
			y += row_h + ROW_GAP;
			row_h = 0.0;
		}
		groups.push(Rect::new(x, y, gw, gh));
		x += gw + GROUP_GAP;
		row_h = row_h.max(gh);
	}
	// `y + row_h` is the last row's bottom; subtracting `top` cancels `scroll`.
	let content_h = (y + row_h - top) + 2.0 * PAD;
	Layout { preview, groups, content_h }
}

/// Scroll range so the last toolbox row can reach the panel bottom.
pub fn max_scroll(body: Rect) -> f32 {
	crate::ui::scroll_max(layout(body, 0.0).content_h, body.h)
}

/// A button's rect within its group's placed origin.
fn button_in(origin: Rect, g: usize, i: usize) -> Rect {
	let group = &GROUPS[g];
	let (col, row) = (i % group.cols, i / group.cols);
	Rect::new(
		origin.x + col as f32 * (BTN_W + GAP),
		origin.y + GROUP_LABEL_H + row as f32 * (BTN_H + GAP),
		BTN_W,
		BTN_H,
	)
}

/// A button's rect in `body` (computes the flow) — the test API.
#[cfg(test)]
fn button_rect(body: Rect, g: usize, i: usize) -> Rect {
	button_in(layout(body, 0.0).groups[g], g, i)
}

/// The button under a click (at the current scroll offset).
pub fn click(body: Rect, x: f32, y: f32, scroll: f32) -> Option<&'static Button> {
	let l = layout(body, scroll);
	for (g, group) in GROUPS.iter().enumerate() {
		for (i, button) in group.buttons.iter().enumerate() {
			if button_in(l.groups[g], g, i).contains(x, y) {
				return Some(button);
			}
		}
	}
	None
}

pub struct ToolboxView {
	pub chrome: UiQuads,
	/// The active tile, drawn through the picker pass (transform applied).
	pub preview: Option<TileQuad>,
}

pub fn view(editor: &EditorState, body: Rect, scroll: f32, w: f32, h: f32, map: SteelMap, hot: Hot) -> ToolboxView {
	let mut q = UiQuads::with_steel_map(map);
	let l = layout(body, scroll);

	// Active tile preview + spec readout (the first flowed block).
	let pr = l.preview;
	q.label("tile", pr.x, pr.y - GROUP_LABEL_H, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
	q.field(pr, w, h);
	let mut preview = None;
	match editor.active_tile() {
		Some(spec) => {
			let project = &editor.project;
			q.label(spec, pr.x + PREVIEW_PX + 6.0, pr.y + 2.0, crate::ui::FONT_SMALL, w, h, theme::INK);
			if let Ok((tile, _)) = project.resolve_ref(spec) {
				preview = Some(TileQuad {
					index: picker::global_index(project, tile),
					transform: tile.transform.bits(),
					rect: Rect::new(pr.x + 1.0, pr.y + 1.0, pr.w - 2.0, pr.h - 2.0),
				});
			}
		}
		None => {
			q.label("none", pr.x + PREVIEW_PX + 6.0, pr.y + 2.0, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		}
	}

	for (g, group) in GROUPS.iter().enumerate() {
		let origin = l.groups[g];
		// Section header: a label with a hairline rule beneath it.
		q.label(group.label, origin.x, origin.y, crate::ui::FONT_SMALL, w, h, theme::INK_DIM);
		q.rect(Rect::new(origin.x, origin.y + GROUP_LABEL_H - 3.0, origin.w, 1.0), w, h, theme::BEVEL.bottom);
		for (i, button) in group.buttons.iter().enumerate() {
			let r = button_in(origin, g, i);
			// A button is "active" when its command reflects current state —
			// the tool, the editor mode, or the picked pass value.
			let active = match &button.act {
				Act::Run(cmd) => {
					(*cmd == "tool pencil" && editor.tool == Tool::Pencil)
						|| (*cmd == "tool picker" && editor.tool == Tool::Picker)
						|| (*cmd == "tool eraser" && editor.tool == Tool::Eraser)
						|| (*cmd == "tool fill" && editor.tool == Tool::Fill)
						|| (*cmd == "tool select" && editor.tool == Tool::Select)
						|| (*cmd == "tool select-rect" && editor.tool == Tool::SelectRect)
						|| (*cmd == "randomize toggle" && editor.randomize)
						|| (*cmd == "layer water" && editor.active_layer_name() == "water")
						|| (*cmd == "layer ground" && editor.active_layer_name() == "ground")
						|| (*cmd == "pass-pick 0" && editor.active_pass == 0)
						|| (*cmd == "pass-pick 1" && editor.active_pass == 1)
						|| (*cmd == "pass-pick 2" && editor.active_pass == 2)
						|| (*cmd == "pass-pick 3" && editor.active_pass == 3)
				}
				Act::Todo(_) => false,
			};
			if let Some(swatch) = button.fill {
				// Pass-type swatch: the semantic pass color, raised like a key
				// (sinking + washing like every other button), amber ring when
				// selected (simple-wrl-editor parity).
				q.rect(r, w, h, swatch);
				q.bevel(r, w, h, 1.0, !hot.pressed(r));
				if hot.pressed(r) {
					q.rect(r, w, h, theme::PRESS);
				} else if hot.hover(r) {
					q.rect(r, w, h, theme::HOVER);
				}
				if active {
					q.border(r, w, h, theme::INK);
				}
				q.label_in(button.label, r, 5.0, crate::ui::FONT_SMALL, w, h, [0.04, 0.05, 0.09, 1.0]);
			} else {
				// Live tools are toggle keys (lit when active); unbuilt tools
				// are plain dim keys.
				match &button.act {
					Act::Run(_) => q.button_active(r, w, h, active, hot),
					Act::Todo(_) => q.button(r, w, h, hot),
				}
				let ink = if matches!(button.act, Act::Todo(_)) {
					theme::INK_DIM
				} else if active {
					theme::ACCENT
				} else {
					theme::INK
				};
				q.label_in(button.label, r, 5.0, crate::ui::FONT_SMALL, w, h, ink);
			}
		}
	}

	// Visible scrollbar when the flow is taller than the panel.
	q.scrollbar(body, l.content_h, scroll, w, h, hot);
	ToolboxView { chrome: q, preview }
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every live button's command must parse (the menu's contract).
	#[test]
	fn every_run_button_parses() {
		for group in GROUPS {
			for button in group.buttons {
				if let Act::Run(cmd) = &button.act {
					crate::command::parse_line(cmd)
						.unwrap_or_else(|e| panic!("{}/{}: {e}", group.label, button.label))
						.unwrap_or_else(|| panic!("{}/{}: empty", group.label, button.label));
				}
			}
		}
	}

	#[test]
	fn buttons_hit_and_groups_dont_overlap() {
		let body = Rect::new(0.0, 600.0, 1280.0, 124.0);
		// Every button is clickable at its own center.
		for (g, group) in GROUPS.iter().enumerate() {
			for (i, button) in group.buttons.iter().enumerate() {
				let r = button_rect(body, g, i);
				let hit = click(body, r.x + r.w / 2.0, r.y + r.h / 2.0, 0.0)
					.unwrap_or_else(|| panic!("{} unclickable", button.label));
				assert_eq!(hit.label, button.label);
			}
		}
		// The preview area hits nothing.
		assert!(click(body, body.x + 30.0, body.y + 40.0, 0.0).is_none());
		// Groups stay inside a 1280-wide dock.
		let last = GROUPS.len() - 1;
		let r = button_rect(body, last, GROUPS[last].buttons.len() - 1);
		assert!(r.x + r.w <= body.x + body.w, "toolbox overflows: {}", r.x + r.w);
	}
}
