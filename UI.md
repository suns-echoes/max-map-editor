# UI - design & implementation

How the editor's interface is built: the design language, the widget kit,
the interaction model, and the rules that keep it consistent. Read
[ARCHITECTURE.md](./ARCHITECTURE.md) first for the big picture; this is the
deep dive for anyone touching `app/src/ui.rs` or any panel/modal.

## Design language

The look aims at the original M.A.X. shell: **brushed gunmetal with neon
accents**, machined rather than drawn.

- **One steel sheet.** Every panel, button, and field is cut from a single
  brushed-steel texture (`app/src/skin.rs`). Docked chrome samples it
  stretched across the viewport - neighbours share one continuous grain;
  floating windows and modals anchor a crop to themselves so the grain
  travels with the window instead of swimming.
- **Materials, not colors.** A chrome fill is a `Material` - a flat base
  tone plus a tinted exposure of the steel grain (`app/src/theme.rs`).
  The stock materials: `PANEL`, `TITLE`, `BUTTON`, `BUTTON_PRIMARY` (warm
  amber - the one confirm action per dialog), `BUTTON_ACTIVE` (green - a
  toggled-on control), `BUTTON_DISABLED` (muted - locked mid-run),
  `TEXTAREA` (dark inset well).
- **Directional bevels.** Light comes from the top-left. Raised controls
  get a lit top/left and shaded bottom/right ring with CSS-style mitered
  corners; inset wells swap the sides. One `Bevel` definition drives all of
  it, blended over whatever is beneath, so the same bevel suits every tint.
- **Embossed text.** Chrome labels draw three times - shadow bottom-right,
  highlight top-left, ink - so type reads as engraved, lit by the same
  light as the bevels.
- **Theme tokens in one file.** Every on-screen color literal lives in
  `app/src/theme.rs`. Re-skinning is a single-file change. The accent is
  the neon green `ACCENT` (#44FF00): titles, selected items, checked
  toggles, progress fills.
- **Two text tiers.** `FONT_BODY` (16 px - menu, titles, primary labels)
  and `FONT_SMALL` (12 px - dense panels, hints, dialogs), both prerendered
  bitmap atlases of the MAX display font (`app/src/font.rs`). The atlas
  covers printable **ASCII only** - an em-dash or `…` silently renders as
  nothing, so UI strings use `-` and `...`.

Layout truth for *what goes where* is `designs/features.drawio` (read
element geometry, not document order); `designs/ui-components-organization.drawio`
maps the component breakdown.

## Where things live

```
app/src/ui.rs          the kit: Rect, Hot, UiQuads (all widget draw helpers),
                       panel/modal chrome, scroll math
app/src/theme.rs       every color/material/bevel token
app/src/text.rs        glyph layout: measuring, ellipsis fitting, word wrap,
                       the GPU text/steel pipelines
app/src/font.rs        baked MAX-font atlases (one per size tier)
app/src/skin.rs        the brushed-steel sheet
app/src/workspace.rs   dockable windows: docking, floating, splitters, drag
app/src/menu.rs        menu bar + dropdowns/submenus + right-click context menu
app/src/tabs.rs        the open-projects tab strip
app/src/<panel>.rs     panel content: picker, palette_panel, toolbox, units,
                       minimap, console
app/src/<modal>.rs     dialogs: newmap, resize, autofix, generator,
                       newfromimage, confirm, errormodal
app/src/modal.rs       the Modal trait routing input to whichever is open
```

Everything is **immediate mode**: each frame, views rebuild their quads from
state into a `UiQuads` batch; nothing is retained between frames. Quads play
back in push order, so z-order is draw order.

## The widget kit (`UiQuads`)

All interactive chrome goes through these helpers - hand-rolling a control
is a bug unless the kit genuinely can't express it:

| helper | what it draws |
| --- | --- |
| `button` / `button_primary` / `button_active` | a raised steel key (plain / amber confirm / green when on) |
| `button_disabled` | the muted locked face - ignores the pointer |
| `toggle_button` | key + checkbox gutter + label, with an `enabled` flag |
| `field` | a dark inset well (text inputs, lists, swatch slots) |
| `progress_bar` | inset well + accent-green fill, optional centered label |
| `scrollbar` | track + thumb, thumb brightens on hover / while dragged |
| `label_in` / `label_fit` | a label in a rect - `_fit` ellipsis-truncates |
| `label_wrapped` | word-wrapped multi-line label |
| `label_emboss` | the raw engraved-text primitive |
| `raised` / `inset` / `material` / `bevel` | the low-level faces the above are built from |

Shared chrome: `ui::panel` (dockable-window titlebar + close),
`ui::modal_frame` + `ui::modal_scrim` (dialog chrome over a 50% veil),
`content_box`/`titlebar_rect`/`body_rect`/`close_rect` for the
borders-as-margin geometry both drawing and hit-testing share.

## The interaction model

### `Hot` - one pointer, every widget

`ui::Hot` is a snapshot of the pointer: the cursor position and where the
primary button went down (while held). The shell (`main.rs`) writes it from
winit events into `EditorState.hot`; every view receives it and every
button face renders from it:

- **rest** - raised bevel;
- **hover** - a darkening wash under the cursor;
- **pressed** - the bevel inverts and the wash deepens: the key visibly
  sinks while held, and lifts without firing if you drag off.

Covered surfaces get `Hot::NONE`: panels and tabs don't highlight under an
open modal, menu dropdown, or context menu, and only the topmost modal
reacts. Headless runs never set a pointer, so screenshots always capture
the rest state - that's what keeps the golden script hashes stable.

Menu rows (dropdowns and the context menu alike) carry their keyboard
shortcut right-aligned in dim ink, resolved once at startup from the
loaded `[Bindings]` - the menu is where shortcuts are discovered, so the
two can never drift apart.

### Click-on-release

Command buttons **arm on press and fire on release-inside**. Dragging off
before letting go cancels the click - the visual (the key lifting un-fired)
matches the semantics exactly.

- *Modals* own their armed button; the `Modal` trait routes the shell's
  release through `on_release`, which fires only if the release still hits
  the armed rect.
- *Panel and tab buttons* defer through an `Armed` value in the shell: the
  action resolved at press is re-checked at release with the same pure
  `click()` hit-test functions, and fires only on an exact match.

Deliberately still press-fired: selections (patterns, anchors, palette
slots, checkboxes), text-field focus, menu navigation, and anything that
*starts a drag* - sliders, scrollbars, map painting, minimap panning,
window moves. Those are immediate by nature.

### Disabled

Settings locked during a live run (generator patterns mid-generate,
conversion options mid-convert, fix modes mid-fix) render the muted
`BUTTON_DISABLED` face: still readable (the checkbox keeps its state),
visibly inert, deaf to the pointer. Lock the input in the press handler
*and* draw the disabled face - both, always.

## Text never escapes its box

The single rule: **dynamic text must be fitted**. Anything whose length the
code doesn't control - file names, project titles, seeds, status lines,
paths - goes through one of:

- `text::fit_label` / `UiQuads::label_fit` - ellipsis-truncate to a width;
- `text::wrap_lines` / `label_wrapped` - word wrap (over-long words are
  char-broken so any width fits);
- structural fixes where truncation isn't enough: menu dropdowns clamp to
  the viewport and submenus flip sides; the tab strip compresses all tabs
  equally (floor 44 px) instead of clipping; the console clips log lines
  and keeps the *tail* of an over-long input visible (the caret end);
  multi-line reports (the generator's) grow their dialog instead of
  cropping.

Static literals may skip fitting, but the fitted call costs nothing when
the text already fits - when in doubt, fit.

## Scrolling & clipping

Scrollable panels reserve `SCROLLBAR_W` on the right, clamp their offset to
`scroll_max(content, view)`, and draw through a GPU scissor rect
(`TextPass::draw_ui_clipped`). Views also cull rows fully outside the clip
window so off-screen content costs nothing. The scrollbar thumb is drawn by
the kit; thumb dragging (grab-point preserved, page-jump on track clicks)
is routed by the shell.

## Adding things - recipes

**A button in a panel:** add the rect to the panel's shared geometry
(drawing and `click()` must use the same function), draw it with a kit
helper passing `hot` through, return an action from `click()`, and decide
its release behavior in the shell - command ⇒ arm it, selection/drag ⇒
press-fire. If its label is dynamic, `label_fit` it.

**A modal:** pure state + geometry in its own file (`dialog_rect` centered,
`drag_offset` for titlebar dragging), `view(w, h, hot)` building a
`UiQuads` over `modal_scrim` + `modal_frame`, `on_press`/`on_release` with
an armed enum for its command buttons, then an `impl Modal` in `modal.rs`
(the `modal_drag!` macro provides the drag plumbing) and an `Option<...>`
slot in `EditorState` + a draw call in `render_frame`. Anything the modal
*does* still goes through a `Command` - the modal only builds the line.

**A theme tweak:** `theme.rs` only. If you're about to write a color
literal anywhere else, stop.

## Testing the UI

Geometry and interaction logic are plain functions, tested as such: hit
tests round-trip against draw rects, press+release flows (including
drag-off-cancels) run per modal, fitting/wrapping have width-budget tests,
and the golden script suite re-renders whole frames headlessly and hashes
the pixels - a visual regression in any chrome fails it.
