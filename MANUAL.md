# M.A.X. Map Editor — Manual

A map editor for *M.A.X.: Mechanized Assault & Exploration* (Interplay, 1996).
This manual covers the portable release; everything also applies when running
from source with `cargo run`.

---

## 1. Getting started

1. Unzip the release anywhere you like. That folder is the whole install —
   the editor reads and writes its settings **beside the binary**, nothing
   touches your home directory.
2. Run `max-map-editor` (Linux) or `max-map-editor.exe` (Windows).
3. Optional but recommended: tell the editor where your M.A.X. game lives —
   open `config/mme.ini` in a text editor and set:

   ```ini
   [Paths]
   MaxPath=/path/to/your/MAX
   ```

   With `MaxPath` set, Load dialogs start in your game directory, and the
   **Units panel** can load the game's unit sprites (see below). Future
   versions will use it for more (opening the MAX folder from the menu,
   installing finished maps straight into the game).

**Linux desktop integration (optional).** The zip includes `install.sh`. It
copies the app to `~/.local/share/max-map-editor` (or a directory you pass),
asks for your MAX path, and adds a menu entry + icons. The editor never
*requires* installation — the unzipped folder works as-is.

The editor opens a green starter map when launched without arguments. To open
a specific document, pass it on the command line or use **File → Open**.

## 2. Documents: projects and WRL

The editor works on **map projects** (`.json`) — layered, tileset-aware
documents. The original game format (`.WRL`) is import/export:

- **Opening a `.WRL`** converts it into a project on the fly.
- **Save** (`Ctrl+S`) writes the project; **Export** bakes a game-ready
  `.WRL` you can drop into your M.A.X. install.

The 24 original maps, rebuilt as ready-to-edit starter projects, ship in
`resources/templates/` — the **Templates** menu loads them directly.

## 3. The workspace

- **Tabs** — several documents can be open at once; the tab strip sits under
  the menu bar. Closing a document with unsaved changes asks first.
- **Panels** — Minimap, Tile Explorer, Color Palette, Toolbox, and Units
  live in docks around the map view. Drag a titlebar to float a panel, drag it near
  an edge to dock it there, drag the splitters to resize. The close glyph
  hides a panel; the **Windows** menu brings it back. **Windows → Reset
  layout** restores the default arrangement.
- The layout is saved automatically on exit and restored on the next start.

### Map navigation

| Action | Default control |
|---|---|
| Pan | drag with Middle or Right mouse button |
| Zoom | mouse wheel (towards the cursor) |
| Fit map to window | `F` |
| Paint | Left mouse button (a drag is one stroke = one undo step) |

## 4. Editing

- **Tile painting** — pick a tile in the Tile Explorer (or eyedrop one from
  the map with the picker tool), then paint. **Fill** floods a connected
  region. With **Randomize** on, painting places random variants of the
  chosen tile so large areas don't look stamped.
- **Layers** — projects have a water base layer and a ground detail layer;
  painting acts on the active one.
- **Auto-shore** — generates correct shore transitions between water and
  land automatically (**Tools** menu; `shore` in the console). The **Fix
  Shore** modal repairs an existing map's shorelines with live stats and a
  Stop button, in three modes: **Fast** (~1 s, re-tiles broken shore only),
  **Aggressive** (unbounded; may also re-tile land next to the shore when
  that closes a seam), and **Destructive** (full freedom over water, shore,
  and land — where no lawful shore exists, the area flattens to water
  rather than keeping a broken seam).
- **Pass editor** — **Mode → Pass** switches to painting passability values
  (the data the game uses for unit movement) with a colored overlay.
- **Resize** — grows or crops the map from any edge (**Tools → Resize**).
- **New from image** — builds a map from any picture: the image is
  quantized to the tileset's palette (with optional dithering) and matched
  to tiles. Great for blocking out a map from a sketch.
- **Generate Random Terrain** — seeds a whole map procedurally
  (**Tools → Generate Random Terrain...**), replacing the current terrain
  entirely (both layers — undo brings the old map back): pick a pattern —
  **Islands**,
  **Continent**, **Land Mass** (always connected), or **River Raid** (land
  cut by rivers) — set the water, obstruction, and decoration percentages,
  pick the shore method (**Auto Shore** for a uniform coastline, **Auto
  Shore ALT** for a more varied one), and press Generate. A progress bar
  tracks the run and the editor stays responsive throughout — the Generate
  button becomes **Abort** while it works, and aborting rolls the map back
  as if nothing happened. Mountains, trees, and other features are stamped
  as **whole multi-tile formations** lifted from the original maps, never
  as random single tiles; decorations are the passable terrain features.
  Coastlines are auto-shored and seam-fixed as part of the run, and the
  whole thing is one undo step. The same seed + settings always produce
  the same map, so a seed is shareable; leave the seed field empty to roll
  a fresh map on every press until one looks right.
- **Selection** — pick the **select** tool (toolbox or **Select** menu) and
  drag over tiles to select them, or the **rect** tool to span rectangles.
  **Shift+drag adds** to the selection, **Ctrl+drag subtracts**, a plain
  drag starts fresh; regions don't have to be contiguous — a thick green
  outline traces whatever is selected. **Select All / Invert / Clear /
  Select Similar** live in the Select menu (`Esc` also clears).
- **Copy / cut / paste** — `Ctrl+C` / `Ctrl+X` / `Ctrl+V` (or the Edit
  menu) work on the selection. Cut clears the selected ground (the water
  base stays); **Clear** (`Delete`, Edit ▸ Clear) does the same without
  touching the clipboard. Paste arms the copied tiles as a **ghost** under
  the cursor — move it where you want, click to place (it stays armed for
  repeat stamping), `Esc` to put it away. Every placement is one undo step.
- **Right-click context menu** — a right *click* on the map (press and
  release in place; holding and moving pans as usual) opens a menu of
  what makes sense right there: cut/copy/delete and template save with a
  selection, paste with a filled clipboard, place/cancel with an armed
  ghost stamp, plus Pick Tile, Center Here, Select All, and Fit Map.
  Click an entry to run it; `Esc`, a click elsewhere, or the wheel closes
  the menu. Menu entries show their keyboard shortcuts, dim on the right
  — the same hints appear throughout the main menus.
- **Templates** — reusable chunks of map. Select something you built,
  open the **Templates Explorer** (Templates menu or **Windows ▸ Dockable
  Dialogs**), and press **save** — the selection becomes a template you
  can stamp on any map that uses the same tile packs. Clicking a template
  arms it as a ghost, exactly like paste. The editor ships **stock
  templates** (formations mined from the original maps) and stores yours
  in `resources/user/templates` as plain JSON — share them, import them
  (**import** / Templates ▸ Import), clone or delete from the same menu.
  Templates whose tile packs aren't in the open map are hidden.
- **Undo/redo** — `Ctrl+Z` / `Ctrl+Shift+Z` (or `Ctrl+Y`), full history.

## 5. The palette

The Color Palette panel edits the project's 256-color game palette:

- click a slot to select it (shift-click selects a range), drag the
  RGB/HSL sliders to retint; **HSL block** operations shift whole ranges;
- the game's **color cycling** (water shimmer, effect sparkles) runs live —
  toggle animation with `A`;
- palettes can be saved/loaded as files; the **saved** tab lists palettes
  shipped with tilesets and your own (kept in `resources/palettes/`) for
  quick hot-swapping;
- **In-Game mode** (View menu) previews the map exactly as the game renders
  it — palette cycling plus 6-bit color; the **CRT** toggle adds a
  scanline/phosphor effect on top, for the full 1996 experience.

## 6. Unit previews

A map's colors only prove themselves with units standing on them. With
`MaxPath` set, **Windows → Units** opens a panel with every unit and
building from your game (loaded straight from MAX.RES — the editor ships no
game art):

- pick a team color (the five swatches in the panel header), click a unit,
  then click the map to stamp it — body, turret, and shadow composited like
  in the game, recolored to the team. A plain click stamps once and returns
  to the pencil; **hold Shift to keep stamping**;
- the **erase** button in the panel header switches to the unit eraser —
  click placed units to remove them one by one (`unit-clear` removes all);
- **View → Show Units** toggles their visibility (picking a unit switches it
  back on automatically);
- placed units follow your palette edits and the live color cycling, so you
  can judge terrain colors against real units while you tune;
- placements are **saved with the project**, so your reference scene is
  there next session — but they never affect the WRL export, and they're
  not part of undo.

Console forms: `unit TAG` / `unit off`, `unit-team red|green|blue|gray|yellow`,
`unit-place TAG X Y`, `unit-erase X Y`, `unit-clear`, `units on|off|toggle`.

## 7. Configuration — `config/mme.ini`

One INI file holds everything. Sections and keys are CamelCase and
**case-sensitive**. The editor rewrites this file when it saves the UI
layout, and **comments do not survive** — this manual is the reference.

### `[Paths]`

| Key | Meaning |
|---|---|
| `MaxPath` | Your M.A.X. game directory. Empty = unset. |

### `[Bindings]` — keyboard

Each entry is `action = chord [chord ...]` — the action is a console
command line (arguments included), the value one or more key chords.
An entry replaces that action's default chords; an **empty value unbinds**
the action; actions you don't list keep their defaults. (The older inverted
`Chord=action` form still loads, with a startup warning.)

```ini
[Bindings]
save-copy backup.json=Ctrl+Shift+B
grid toggle=G F8
fit=
```

Chords: optional `Ctrl` / `Shift` / `Alt` plus one key — letters, digits,
punctuation, `F1`–`F12`, `Escape`, `Enter`, `Space`, `Tab`, `Backspace`,
`Delete`, `Insert`, `Home`, `End`, `PageUp`, `PageDown`,
`ArrowLeft/Right/Up/Down`, `Backquote`, `Plus`, `Minus`, `Equals`.

Bound actions show their chord **in the menus**, right-aligned and dim.
One chord may serve several actions with disjoint contexts — out of the
box the digit keys pick pass values in the Pass Table Editor and zoom
presets in the map editor; the tool keys only act in the map editor.

Default bindings:

| Action | Keys | |
|---|---|---|
| `save-project` | `Ctrl+S` | save (asks for a path if never saved) |
| `file-dialog save-as` | `Ctrl+Shift+S` | Save As |
| `file-dialog load` | `Ctrl+O` | Load Map |
| `new-map` | `Ctrl+N` | New Map modal |
| `close-project` | `Ctrl+W` | close the active tab |
| `export` | `Ctrl+E` | bake a game-ready WRL |
| `undo` / `redo` | `Ctrl+Z` / `Ctrl+Shift+Z`, `Ctrl+Y` | |
| `cut` / `copy` / `paste` | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | clipboard (§4) |
| `delete` | `Delete` | clear the selected ground (Edit ▸ Clear) |
| `select all` / `select clear` / `select invert` | `Ctrl+A` / `Ctrl+D` / `Ctrl+I` | |
| `tool pencil` / `eraser` / `picker` / `fill` | `B` / `E` / `I` / `K` | map editor only |
| `tool select` / `tool select-rect` | `L` / `M` | map editor only |
| `fit` | `F` | fit the map in the view |
| `zoom-to 1` / `0.5` / `0.25` | `1` / `2` / `3` | map editor zoom presets |
| `zoom 1.25` / `zoom 0.8` | `Plus`, `=` / `Minus` | zoom in / out |
| `pass-pick 0`–`3` | `0`–`3` | Pass Table Editor: pick the pass value |
| `grid toggle` | `G` | cell grid overlay |
| `pass-overlay toggle` | `O` | pass-value overlay |
| `units toggle` | `U` | show/hide unit previews |
| `animate toggle` | `A` | palette cycling |
| `console toggle` | `Backquote`, `F1` | |
| `quit` | `Escape` | see below |

`Escape` is layered: it first closes an open menu or context menu, then
disarms a ghost stamp, then clears the selection — only an idle `Escape`
asks to quit.

### `[Mouse]`

| Key | Meaning | Default |
|---|---|---|
| `PanButtons` | space-separated buttons that drag-pan (`Left` `Middle` `Right`) | `Middle Right` |
| `PaintButton` | button that paints | `Left` |
| `ZoomStep` | wheel zoom factor per notch, `1.01`–`2.0` | `1.15` |

A right **click** (no drag) over the map always opens the context menu
(§4), whether or not `Right` is among the pan buttons.

### `[Workspace]`

Machine-written — the saved panel layout. Edit at your own risk; deleting
the whole section resets the layout to defaults.

## 8. The console

`` ` `` (Backquote) or `F1` opens the in-app console. Every editor action is
a console command — the same commands work in `[Bindings]` and in script
files, so anything you can click, you can also type, bind, or automate.

Commonly useful:

| Command | Does |
|---|---|
| `open PATH` / `save [PATH]` / `export [PATH]` | document I/O (`export` bakes a `.WRL`) |
| `new W H PACK SEED` | new map (e.g. `new 64 64 GREEN 7`) |
| `tile SPEC` / `paint X Y` / `fill X Y` | choose a tile and place it |
| `shore` | run auto-shore |
| `generate PATTERN [water=N] [obstructions=N] [decorations=N] [seed=N] [shore=sweep\|alt]` | random terrain (§4) |
| `select all\|clear\|invert\|similar`, `select-rect X0 Y0 X1 Y1 [add\|sub]` | selection (§4) |
| `copy` / `cut` / `paste` / `delete`, `stamp X Y`, `stamp cancel` | clipboard + ghost placement |
| `context-menu X Y` / `context-menu off` | open/close the right-click menu (scripts) |
| `template-save [NAME]`, `template-pick NAME`, `template-delete`, `template-clone` | templates (§4) |
| `undo` / `redo` | history |
| `zoom-to N` / `pan-to X Y` / `fit` | view |
| `grid on|off|toggle` | cell grid overlay |
| `animate`, `ingame`, `crt` | toggles (palette cycling, in-game look, CRT) |
| `mode map|pass`, `layer water|ground` | editing mode / active layer |
| `unit TAG`, `unit-team NAME`, `unit-place TAG X Y`, `unit-clear` | unit previews (§6) |
| `window ID on|off`, `dock ID left|right|top|bottom|float` | panel layout |
| `save-settings`, `reset-layout` | persist / reset the UI layout |
| `screenshot PATH` | save a PNG of the current frame |
| `quit` / `quit!` | exit (`!` discards unsaved changes) |

There are more — including `assert-*` commands used by the regression
scripts; see them in action under `scripts/` in the repository.

## 9. Command line & scripting

```
max-map-editor [MAP.WRL|PROJECT.json] [options]

--script FILE       run commands from FILE (one per line, # comments)
--screenshot OUT    shorthand: render headless and save a PNG
--crop x,y,w,h      crop the --screenshot to a region
--resize WxH        resize the --screenshot after cropping
--headless          run the script without a window, then exit
--size WxH          render-target size (default 1280x800)
--settings FILE     load/persist all settings from FILE (an alternate mme.ini)
```

A script file is just console commands, one per line. Scripts double as
regression tests in the repository — they can assert map state
(`assert-cell`, `assert-hash`, `assert-dirty`) and fail the run when the
editor misbehaves.

## 10. Where things are stored

| What | Where |
|---|---|
| Settings, bindings, UI layout | `config/mme.ini` beside the binary |
| Tilesets (tile packs) | `resources/assets/<PACK>/` |
| Starter projects (the 24 originals) | `resources/templates/` |
| Your saved palettes | `resources/palettes/` |
| Default save location for maps | `resources/maps/` (created on first save) |

A tile pack is a folder of palette + tile data + passability + variant
groups; projects reference packs by name and carry their own palette, so a
map and its look travel together. Custom tile packs are planned — the format
will be documented once it stabilizes.

---

M.A.X. COPYRIGHT © 1996 INTERPLAY PRODUCTIONS. ALL RIGHTS RESERVED.
The editor ships no original game content — point `MaxPath` at your own copy.
