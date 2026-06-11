# Architecture

A quick tour for anyone reading or hacking on the code.

## The one invariant

**Every mutation flows through a `Command` executed by
`EditorState::execute`** (`app/src/state.rs`). Interactive input, `--script`
files, key bindings, and the in-app console all build `Command`s
(`app/src/command.rs`) and run them through that single mutator; it returns
an `Outcome` the shell acts on (redraw, rebuild renderer, screenshot, quit,
fail).

This is what makes the editor replayable, scriptable, and headless-testable
— a session is just a list of commands, and a list of commands is a test.
New behavior means a new `Command` variant plus an arm in `execute`, never a
side-channel mutation.

## Workspace layout

```
app/                 the binary
  src/main.rs        winit shell, event routing, frame composition
  src/state.rs       EditorState + execute() — THE mutator
  src/command.rs     Command enum + the script/console parser
  src/input.rs       key/mouse bindings (config/mme.ini [Bindings]/[Mouse])
  src/shaders/       WGSL shaders
  src/*.rs           draw passes (project_render, blit, minimap, text, grid,
                     crt) and immediate-mode UI (ui, workspace, menu, picker,
                     palette_panel, toolbox, console, modals…)
crates/
  map-core           pure-logic document kernel — no GPU, no winit:
                     Project (the in-memory document), patch-based undo/redo,
                     palette ops, WRL bake, the auto-shore solver (shore.rs)
  max-assets         decoders for the original game formats (WRL, RES, images)
  ini, json          small hand-rolled parsers (ini is shared with sister
                     projects and copied, not path-linked)
config/mme.ini       all settings: paths, bindings, mouse, saved UI layout
resources/           runtime data: tile packs (assets/), starter projects
                     (templates/), palettes, icons, UI skin
scripts/             golden regression scripts (see Testing)
```

## Document model

`Project` (`crates/map-core/src/project.rs`) is **the** in-memory document:
layered (water base + ground detail), tileset-aware, carrying its own
palette. The original `.WRL` format is import/export only — opening a WRL
converts it to a `Project` on the fly, and Export bakes a `Project` back
into a game-ready WRL.

The bake is proven byte-exact: the test suite re-bakes converted projects
and compares them against all 24 original maps (modulo palette-cycled
pixels, which the game animates anyway).

## Rendering

wgpu passes composed in `main.rs::render_frame`, shared by the live window
and the screenshot path — captures are always faithful. The map is drawn
from cell data on the GPU (`project_render.rs`); UI is immediate-mode quads
+ a bitmap font; In-Game mode adds palette cycling + 6-bit color, and the
CRT pass post-processes the whole frame into an offscreen target.

## UI

Immediate-mode over a shared widget kit (`app/src/ui.rs`): brushed-steel
materials + directional bevels from one theme file, a pointer snapshot that
gives every control hover/pressed/disabled states, click-on-release on
command buttons (drag off to cancel), and fitting rules so dynamic text
never escapes its container. Headless captures render with no pointer, so
golden screenshot hashes don't depend on the mouse. The full tour —
design language, kit, interaction model, recipes — is in [UI.md](./UI.md).

## Testing

```sh
cargo test --workspace
```

- **Unit/integration tests** live with their crates; `map-core` is fully
  headless.
- **Golden scripts** (`scripts/*.script`) replay real editor sessions
  through the actual binary with `--headless`, asserting document hashes
  and cell contents along the way (`app/tests/scripts.rs` runs them all).
  They need a GPU adapter, so they self-skip in CI.
- **Equivalence proofs** compare composition and bake output against the
  original game maps. Those maps are copyrighted and not in the repo —
  `tools/fetch-testdata.sh MAX_DIR` copies them from your own install into
  the gitignored `testdata/originals/`; without them these tests skip
  loudly.

## House rules

- `cargo fmt` before committing — `rustfmt.toml` (hard tabs, 120 cols) is
  the source of truth; CI fails on drift.
- First-party crates are clippy-gated with `-D warnings` (allow-list in the
  root `Cargo.toml`); the copied crates track their upstreams and aren't.
- Dependencies are minimal, exact-pinned, and discussed before adding —
  prefer std and hand-rolled.
