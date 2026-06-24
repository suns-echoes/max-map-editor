# Optimization backlog

Living checklist from a 5-way code audit (2026-06-20). Tick items as done so work
can resume cleanly across sessions. Effort: S/M/L. Each item is ~one PR-sized unit;
keep `cargo fmt`/`clippy`/`test` green after each. Line numbers are from the audit
snapshot — re-locate before editing.

> Source: full plan + rationale in `temp/optimization-plan.md`. Known unrelated
> failure: `SNOW_5` equivalence test (pre-existing, map-core/testdata).

## Leave alone (verified good — do NOT "fix")
- parse↔execute split (`command.rs` parses, `state.rs::exec_*` executes).
- `workspace.rs` (docking) + `input.rs` (key binding) — well-factored, tested.
- App-layer error flow (`Outcome::Failed(String)` → `ErrorModal`) — consistent.
- Core/IO boundary in map-core (except shore.rs env/eprintln — see P0).
- `json` crate (depth guard + adversarial test); `select.rs` shared dropdown.
- `worldgen.rs` length — cohesive single pipeline; long ≠ wrong.

---

## Phase 0 — Safety & quick wins (DONE 2026-06-20)
- [x] **Bounds-check asset decoders** — `.get()`/length guards on unbounded reads.
      `multi.rs` (count bytes + row/frame offset tables → `data.get(..)?`), `big.rs`
      (palette-size guard + RLE opcode/literal/repeat bounds + `i16::MIN` overflow),
      `color.rs` (palette lookups → `.get()`, transparent fallback). **DONE.**
- [x] Truncation/malformed tests — 14 new tests (multi/big/color), incl. "truncate at
      every length never panics" + oversized-image-count + `i16::MIN` count. **DONE.**
- [x] `worldgen.rs:767` `partial_cmp().unwrap()` → `total_cmp` (NaN panic). **DONE.**
- [x] 3 known clippy fixes: `palette.rs` (`//!` doc), `big.rs` (`repeat_n`),
      `simple.rs` (`SimpleImageRaw` type alias). **DONE — `clippy -p max-assets` clean.**
- [x] **Feature-gate** shore instrumentation behind default-off `shore-instrument`
      (map-core/Cargo.toml). `SHORE_TIME` timing + `SHORE_REPAIR_BUDGET` override now
      `#[cfg(feature)]`; default build is env/stderr-free (`mark` becomes a no-op
      closure; `budget` uses the computed default). Both feature states build + test
      clean. To profile: `cargo run --features map-core/shore-instrument`. **DONE.**
- [x] Command routing safety net — `state.rs::toolbox_commands_route_without_panicking`
      executes every side-effect-free toolbox `Act::Run` command on a scratch
      `EditorState` (a mis-routed variant trips `unreachable!` → test fails). Full
      exhaustiveness deferred to the Phase 4 routing-table refactor. **DONE.**

> Phase 0 verified green: `cargo fmt --check` clean, `clippy --workspace` clean (3 old
> max-assets warnings gone), full suite passes — ini 45, json 5, map-core 101,
> max-assets 27, app 180. Only red is the known unrelated SNOW_5 equivalence
> (2/12544 cells). NOT committed.

## Phase 1 — Structure / de-silo (COMPLETE 2026-06-20)

> All substantive items done: the 15→1 modal-field collapse + delegation macros,
> the `project.rs` directory-module split, the `FileDialog` path-policy extraction
> (+test), the `pack.rs` reader-injected `load` split (+in-memory error tests), and
> the 4-cluster `EditorState` de-silo (PaletteManager/TemplateLibrary/TabSet/TileOps,
> 83→55 fields). The two remaining items are **optional file-splits deferred by
> design** (shore = cosmetic reorg of a cohesive module; worldgen = the audit's own
> "leave alone"). Every step behavior-preserving: app 193 tests + headless render +
> map-core 107 lib + equivalence byte-identical (only SNOW_5) + clippy/fmt clean.
> This-session commits: c4df8f6 (FileDialog), 51b06e0 (pack), f3048a6 / 618bee5 /
> 5484d3e / 739bdba (the 4 EditorState clusters).

- [x] **Collapse 15 `Option<T>` modal fields → one `Option<Box<dyn Modal>>`.**
      `active_modal`/`active_modal_ref`/`close_modal` → one-liners; "one modal at a
      time" is now a type invariant. Trait gained `as_any`/`as_any_mut`/`into_any`
      (via `modal_glue!`, since Rust 1.85 < trait-upcasting); `EditorState` gained
      `modal_as`/`modal_as_mut`/`take_modal_as`/`open` helpers; render block → one
      downcast chain preserving per-arm draw order. Behavior-preserving (180 app +
      101 map-core green, 5 arms rendered headless). **DONE on branch `modal-collapse`
      (2c9768d), −83 net lines.** (Render handled via downcast, not a `Modal::render`
      default - kept GPU code in main.rs.)
- [x] **Collapse the repetitive `Modal` delegation methods** behind two macros
      parameterized by the modal's `from_*` mapper: `text_modal_glue!($from)`
      (edit_context + on_press + on_release + on_drag, 7 text modals) and
      `modal_press!($from)` (on_press + on_release, 3 confirm modals). `modal.rs`
      825→741. **DONE.** Deliberately kept per-modal `on_key` (genuinely varies) and
      the `from_*` mappers as-is rather than forcing them into a macro - the audit's
      ~300-LOC target assumed absorbing those, which would hurt readability.

> **Found + fixed (pre-existing, unrelated):** 8 of the headless `scripts/*.script`
> files referenced the stale `resources/templates/GREEN_1.json` (moved to
> `assets/maps/` in restructure 85097eb), so `scripts_pass_headless` had been red.
> Repointed all 9 to `assets/maps/` - the script suite is green again.
- [x] Extract cohesive clusters out of `EditorState` into sub-structs (done
      incrementally, one cluster per commit, each behavior-preserving): **PaletteManager**
      (`palettes`: sel_end/multi/scroll/wrl_scroll/show_saved/files/sel), **TemplateLibrary**
      (`templates`: entries/scroll/sel/cell/dropdown_open), **TabSet** (`tabs`: slots/active/
      replace_scratch), **TileOps** (`tile_ops`: dirty_packs/clipboard). 17 fields regrouped
      into 4 sub-structs; ~150 access sites rewired (scripted, word-boundary-guarded);
      cross-cutting `active_color`/`clipboard`/`stamp`/`selection` deliberately kept on the
      editor. EditorState dropped to **55 fields**. Verified each step: app builds + 193
      tests + headless render + clippy/fmt clean. **DONE.**
- [x] Split `project.rs` (2892) → directory module `project/`: `mod.rs` (2307, the
      document model), `serde.rs` (443, `from_str_in` + `save_string` in an
      `impl Project` child module via `use super::*`), `palette_reimport.rs` (163,
      the `PaletteReimport` session; re-exported from mod.rs). `git mv` (reversible).
      Behavior-neutral: fmt/clippy clean, equivalence corpus + serde round-trips +
      all 180 app tests pass (only SNOW_5 red). Optional further `transform/wrl/
      compose` sub-splits deferred. **DONE 2026-06-20.**
- [x] Extract `FileDialog` path policy → free fns `dialog_default_dir(purpose,
      resources_root, doc_path, max_path, user_maps)` + `dialog_suggested_name(purpose,
      doc_path, project_name)` (pure, no rfd, no `EditorState`); the `exec_io` arm's
      ~55-line inline `start`/`suggested` blocks collapse to two calls. Unit-tested
      (`dialog_path_policy_follows_purpose`: palette/template/load/save dirs + create-on-
      first-use + `.json` name policy) — 193 app tests, clippy/fmt clean. **DONE.** (The 2
      `unreachable!` are kept — they're correct exhaustiveness guards for the PNG variants
      handled before the json dialog; a catch-all `_` would be less safe.)
- [defer] (Optional) Split `shore.rs` 3 solvers → `shore/{mod,sweep,walk,fix}.rs`.
      **Deferred by design.** Pure file reorganization (no logic change); shore.rs's
      *length* was never a flagged problem (the only shore P0 was the env/eprintln, already
      feature-gated), the three solvers share a dense web of types (`Trial`/`Nb`/`Cell`/
      `Family`) and free fns (`comp_*`/`dir_ok`/`lawful_band_transforms`), and the Phase 2
      dedup already tightened it. Cosmetic split of a cohesive (if large) module = effort +
      module-boundary risk without a clear payoff.
- [defer] (Optional) `worldgen.rs` submodule split. **Deferred by design — contraindicated
      by the audit's own "Leave alone" note** ("worldgen.rs length — cohesive single
      pipeline; long ≠ wrong"). The noise/stamp/session stages are one deterministic
      pipeline; splitting files wouldn't improve it. (The genuine dedup here — sharing
      `splitmix` and `flood4` — was already done in Phase 2.)
- [x] `pack.rs` `load`: split the parse/validation body into `TilePack::from_reader(name,
      read_bin, read_text)` (reader-injected, no fs); `load(assets_root, name)` is now a
      thin wrapper supplying fs closures. Unblocks error-path tests without on-disk
      fixtures — added `from_reader_loads_in_memory_and_surfaces_errors` (valid load + bin
      %4096 + count mismatch + object-form out-of-range + missing required json + bad
      optional sidecar). Behavior-preserving: equivalence corpus byte-identical (only
      SNOW_5), 107 map-core lib + clippy/fmt clean. **DONE.** (`dump` was already pure
      serializers — `bin_ordered`/`ids_json`/`colors` — over thin `write_if_changed`, so
      no split needed there.)

## Phase 2 — Duplication kills (COMPLETE 2026-06-20)

> All duplication-removal items done or deliberately resolved. map-core fully
> byte-verified (equivalence corpus 23/24 — only the pre-existing SNOW_5 — plus
> shore ground-truth + worldgen golden vectors); app changes verified by 192 unit
> tests + the headless render suite; every step clippy/fmt clean. Two items are
> recorded as **deliberate non-actions** with rationale rather than forced: the
> cross-panel `Toolbar` unification (divergent per-panel layouts, no byte-identical
> verification) and `ButtonRow` (per-modal interaction logic Phase 1 kept explicit).
> Commits: 93a06e7, e97330c, a80f9df, 314683c, 431b63a, 612f8a6, af20e7a, ac3d60d,
> 7bbbc1e, 1bd30fe, d1b9341, fd51777.

- [x] **Shared scrolling-`Grid` module** — new `app/src/cellgrid.rs` (`Grid`
      parameterized by cell/header/gap/pad + a per-row name-strip `row_extra`) owns
      `cols`/`item_rect`/`rows`/`max_scroll`/`content_height`/`index_at`/`scissor`.
      picker/units/templates_panel keep their function signatures as thin delegating
      wrappers (zero call-site churn); picker's inverse hit-test → `Grid::index_at`.
      `palette_panel` left as-is (different fixed-column palette-section model).
      Behavior-preserving (180 app tests, 3 grids render headless). **DONE.**
- [skip] `ButtonRow` (arm-on-press / release-fire-inside / click-out). **Assessed and
      deliberately not forced.** Only ~4-5 *pure-button* modals (confirm/deletetemplate/
      palettedelete/dedupetemplates/errormodal) share the simple arm/fire kernel; the
      rest of the "13" interleave text fields, sliders, dropdowns, and multi-stage
      confirms (e.g. palettename's overwrite-arming computes its `Press::Run` via
      `try_confirm`, not a static action) that a generic row can't express. Each modal's
      `on_press`/`on_release` is ~8 simple lines and the consequential ones are unit-
      tested (e.g. palettename `confirm()`); a generic `ButtonRow` would be a behavioral
      rewrite of core modal interaction with only headless-screenshot coverage (no byte-
      identical verification) - exactly the per-modal interaction logic Phase 1 chose to
      keep explicit (cf. `on_key`/`from_*`). The `main.rs` `Armed::*` dispatch is 8
      panel variants with divergent fire logic, same conclusion. Low reward, real
      interaction-regression risk - left explicit by design.
- [x] `ui::button_pair()` + modal chrome. Added `ui::button_pair(d, w, pad, btn_h) ->
      (left, right)` + `ui::MODAL_BTN_W` const; the byte-identical Cancel+confirm pair
      formula in 5 modals (palettename/dedupetemplates/deletetemplate/palettedelete/
      renametemplate) now delegates (metrics passed in → behavior-preserving). App builds
      + 192 unit + headless + clippy/fmt clean. **DONE.** (The `modal_chrome` opener
      already exists as `ui::modal_frame` + `ui::modal_scrim`. The shared **metrics
      module** is NOT applicable — `TITLE_H`/`BTN_H`/`PAD`/`GAP` are per-modal *tuned*
      values (TITLE_H 22 vs 24; BTN_H 20/22/23/24; PAD 4/8/10/12/16), not one repeated
      constant, so unifying them would change layouts.)
- [x] `TextInput::edit_context()` — one method on `TextInput` builds the
      `EditContext { has_selection, is_empty }`; all 9 modals' `edit_context` now
      delegate (`Some(self.input.edit_context())` / `field.edit_context()`), so the
      struct literal lives in exactly one place. Promoted `clipboard_get/set` to
      `pub(crate)` and routed generator.rs's seed-copy through `clipboard_set` (drops
      its duplicate `arboard::Clipboard` block). App builds + 192 tests + clippy/fmt
      clean. **DONE.**
- [x] Palette/color primitives. DONE: `color::parse_hex_rgb`/`rgb_to_hex` (5 sites
      across palette/pack/serde, +test, equivalence corpus byte-identical); hoisted
      `theme::srgb_to_linear` (2 dup copies → 1). DONE (this pass): `palette::slot_rgb`/
      `set_slot_rgb` accessors (exported) replace the `slot as usize * 3` /
      `pal[at..at+3]` triples across image_import/palette_convert/palette_reimport/
      serde/game_palette; named-range consts `ANIMATED_SLOTS` (9..=31) / `WATER_SLOTS`
      (96..=127) (exported) replace the bare literals in those modules' predicates.
      Behavior-preserving: 23/24 equivalence maps byte-identical (only SNOW_5),
      map-core 106 lib + clippy/fmt clean. (Test-local usize-indexed literals left
      as-is — casting to u8 consts adds noise without value.)
- [x] Render helpers. `render::load_pass(encoder, target, label)` replaces the 7
      identical `begin_render_pass` load/store single-attachment descriptors (blit/grid/
      units_render/crt/text/project_render×2); `TILE_PX` dup const in project_render.rs
      → `use crate::render::TILE_PX`. App builds + 192 unit + headless-render (all GPU
      passes) green + clippy/fmt. **DONE.** (`srgb_to_linear` was already single-source
      in theme.rs. Deliberately left: `nx`/NDC closures are *not* uniform — text rounds,
      the others don't, and they're trivial 1-liners; the `scissor_px` clamp idioms are
      **deliberately divergent** per the audit, so unifying them wouldn't be behavior-
      preserving; `rgb⇄rgba` is a 1-line `extend` over differently-shaped sources.)
- [x] **Move file IO out of `exec_*` → pure `Result`-returning helpers.**
      - `app/src/palette_io.rs` — palette save/rename/delete/import/load (+5 tests),
        success tail → `EditorState::palette_saved`.
      - `app/src/settings_io.rs` — `save_workspace` (the `SaveSettings` INI
        re-read/merge/write) (+2 tests: writes `[Workspace]`, preserves hand-edited
        sections).
      - Template save/clone/import/rename tail → `EditorState::template_saved`.
      `write_project` was already a `Result` helper; `Template::save`/`load` already
      own the template file IO. **DONE** — 187 app tests, all the IO now unit-tested
      in isolation (also seeds Phase 3).
- [x] `on/off/toggle` parse helper — `on_off_toggle(args, verb)` routes all 11 flag
      commands; unified `animate`+`console` to bare=toggle like the rest. command.rs
      −44 lines, 187 app tests. **DONE.**
- [x] `req_str`/`opt_str` arg helpers beside `num()` (`command.rs`). `req_str(args,
      i, msg)` (custom messages preserved — the call sites carry good ones) replaces
      19 `…ok_or("msg")?.to_string()` + 7 `PathBuf::from(…ok_or("msg")?)`; `opt_str(
      args, i)` replaces 4 `…map(|s| s.to_string())`. App builds + 192 tests + clippy/
      fmt clean. **DONE.**
- [x] `Project::push_undo` helper — the 8 copied `undo_stack.push(p); if len >
      MAX_UNDO { remove(0) }` blocks → one `self.push_undo(p)` (map-core 106 lib +
      clippy/fmt clean). **DONE.** (`with_content` constructor evaluated + **not done**:
      the 3 constructors share only a 6-field *transient tail* — the ~17 content fields
      genuinely differ — so a positional/struct `with_content` just relocates the field
      names (net-neutral), and grouping the transients into a sub-struct is ~69-site
      churn, disproportionate to removing ~12 trivial lines.)
- [x] `flood4` — new `crate::grid::flood4(w, h, start, seen, pred, visit)` (caller-
      owned `seen` so component-labeling shares one buffer). Replaces all 3 4-connected
      flood copies: `Project::fill` (indices collected in pop order, variant/rng rolled
      after so rng order is byte-identical), worldgen `connect_land` (shared `seen`,
      `comp` labeled in `visit`), and the land-connectivity test. Push order
      [L,R,U,D]+LIFO preserved → deterministic output unchanged; map-core 106 lib +
      clippy/fmt clean. **DONE.**
- [x] shore.rs duplication. Free fns at module scope: `dir_ok` (2 identical closures),
      `comp_admits`/`comp_cseam`/`comp_tseam` (the `comp`-backed seam scoring — method
      `cseam`/`tseam` now delegate, the destructive closure + 2 fix-solver `admits`/
      `cseam` closures collapse to one-liners), and `lawful_band_transforms` (the
      `for fam/rot/mirror { dir_ok… }` candidate enumeration, ×3). −38 net lines.
      Verified byte-identical: 106 lib + 17 shore unit + 2 shore-ground-truth +
      equivalence (only SNOW_5) + clippy/fmt clean. **DONE.** (Kept in one `shore.rs` —
      the optional file-split is a separate Phase 1 item.)
- [~] Panel header flow. DONE: `templates_panel.rs`'s `header_layout`/`header_height`
      exact-dup flow loop → one `flow_header(body, emit)` (closure-driven; `header_height`
      passes a no-op so it keeps its no-allocation property). Behavior-preserving: 192
      unit + headless (templates explorer) + clippy/fmt clean. REMAINING (deferred, risky):
      the cross-panel `Toolbar` draw+hit unification across toolbox/palette/templates/menu
      — those four toolbars have structurally different controls, draw, and hit logic, so
      a single helper would be a large layout/hit-test rewrite with only headless-screenshot
      coverage (no byte-identical verification). **(M)**
- [x] Dedupe the multi-arg `panel::click(...)` calls at press + release. Three `&self`
      hit-test helpers — `palette_click` (10-arg `palette_panel::click`), `wrlpalette_click`
      (7-arg `click_bare`), `templates_click` (`templates_panel::click`) — bundle the
      editor/modifier state both the arm (press) and the re-hit (release/`fire_armed`)
      sites fed in, so the two call sites can't drift. Cursor passed explicitly (no
      behavioral assumption). 6 call sites → one-liners; app builds + 192 unit + headless
      render (palette/templates panels) + clippy/fmt clean. **DONE.**
- [x] map-core JSON / shared helpers. `read_json_opt(file)` closure in `pack.rs::load`
      collapses the 6 optional-sidecar read→parse→prefix-error envelopes (palette/pass/
      match/variants/props/patterns). `splitmix(z)` free fn beside `Rng` — `Rng::next_u64`
      and worldgen's `lattice` (was `mix`) now share it (golden vectors confirm bit-
      identical). `check_map_size(w, h)` replaces the 4 `w==0||h==0||w>1024||h>1024`
      guards (project new/resize, serde load, template — `MAX_DIM` const). `encode_cell_grid`
      shares the JSON map-body row writer between project `save_string` and template.
      map-core 106 lib + equivalence byte-identical (only SNOW_5) + clippy/fmt clean.
      **DONE.** (The `json_ext` `obj`/`u8_field` micro-helpers skipped — the
      `.as_object().ok_or(format!("{file}: …"))` messages are per-file and a generic
      accessor would lose that context for little gain.)
- [x] Seed-roll helper `roll_seed()` (3× `SystemTime` block) → `fn roll_seed() -> u64`
      (`seed.unwrap_or_else(roll_seed)`); `check_pass(value, verb) -> Option<Outcome>`
      replaces the 3× `if value > 3 { return Outcome::Failed(...) }` pass guards.
      App builds + 192 tests + clippy/fmt clean. **DONE.** (The `Outcome::Failed`
      prefix-helper sub-item deliberately **skipped**: 141 sites, messages too varied
      for a clean `(verb, err)` prefix, and a blanket `failed()` wrapper would hurt
      grep-ability of the explicit failure-exit constructor.)

## Phase 3 — Missing tests (COMPLETE 2026-06-20)

> Test coverage filled across map-core, json, and the app. Feasible error/edge
> paths are covered; two guards are noted infeasible to unit-test (bake's >65535-
> tile overflow and image_import's identical one — both need impractically huge
> inputs). All green: map-core 117 lib + json 8 + app 211 + the integration suites.
> This-session commits: d31fe03, 65eadc4, f99f5e7, 2f996e5, a5f56fe.

- [x] **`exec_*` execution tests** — drive `EditorState::execute(cmd)`. DONE: the
      exec-layer item (nav/overlay/select/set-color/erase) plus `paint_fill_transform_
      and_hsl_drive_state` (paint places, fill floods, transform rotates the brush /
      4× cw = identity / bad op fails, hsl-block darkens a dynamic slot + refuses a
      static one).
- [x] **File-IO round-trips + error paths** (temp dirs) — DONE: `palette_io` (5) +
      `settings_io` (2) earlier; `save_then_open_round_trips_the_project_on_disk` (a
      painted + palette-overridden project saves, clears dirty, reloads to an identical
      hash); `free_stem_in_bumps_on_collision` (collision bump + exclude-frees-base).
- [x] **worldgen golden noise vectors** — `noise_primitives_are_pinned` pins
      `mix`/`value_noise`/`fbm`/`field_at`/`rotated` to reference values (+ identity/
      range invariants), so float drift fails loudly instead of silently re-rolling
      maps. **DONE.** (`GenSession::new` validation now covered too —
      `gen_session_rejects_out_of_range_percentages`, boundaries 0/100 accepted.)
- [x] **exec-layer state tests** (the audit's #1 gap) — `state.rs` now drives
      `execute()` and asserts state for nav (pan/zoom/fit), overlay (grid/animate/
      crt/pass-overlay toggles), select (all/clear/invert/unknown), set-color
      (dynamic written / static refused), erase. **DONE** (5 tests, +file-IO tests
      from `palette_io`/`settings_io` earlier).
- [x] `game_palette::apply_game_statics` boundary — sentinel-fill test: 64..=159
      kept, every static slot = the in-game baseline, no FF00FF leak. **DONE.**
- [x] `select_similar` — built a 4×4 GREEN `Project`, placed two ground tiles, and
      covered both branches (selection-derived keys + fallback) and the no-op. **DONE.**
- [x] `pack.rs` loader error surface — DONE via the reader-injected `from_reader`
      (Phase 1): `from_reader_loads_in_memory_and_surfaces_errors` (bin %4096, count
      mismatch, object-form out-of-range, missing json, bad sidecar) +
      `from_reader_surfaces_sidecar_validation_errors` (pass unknown-tile / OOR value,
      props bad type / transformable, patterns size / unknown-tile).
- [x] `project.rs` load rejections — DONE. Header guards (`load_rejects_malformed_
      headers`) + `load_rejects_malformed_body` (row/cell count, non-string scalar/array
      cell, pass row count/len, unit OOR, palette-owner count) +
      `load_accepts_legacy_sparse_pass_and_positional_overstack` (3-ref >MAX_LAYERS
      positional fallback + legacy `{x,y:val}` pass form + OOR).
- [x] `image_import.rs` degenerate inputs — DONE: empty source / bad rgba length / map
      size, `pin` after step, single-colour histogram, `Coverage::Crop`/`Fill` reach the
      requested size. (The >65535-unique-tile overflow guard exists but needs an
      impractically huge image to trip — left unexercised.)
- [x] `confirm.rs` — DONE: Purpose → command lines, arm-on-press/fire-on-release for all
      three buttons, release-off disarms, click-out cancels.
- [x] `preferences.rs` — DONE: `from_project` → `values` round-trip of all six info
      fields; players segmented control select-then-toggle-off.
- [x] `deletetemplate.rs`/`dedupetemplates.rs` — DONE: dedupe Remove live + dialog taller
      only when `has_dupes`, Cancel arm/fire + click-out; delete Delete/Cancel arm/fire,
      release-off disarms, click-out cancels.
- [x] `FileDialog` `default_dir`/`suggested_name` (`dialog_path_policy_follows_purpose`,
      Phase 1); `free_stem_in` collision bump; `template_pack` selection (prefers non-WATER
      → WATER → MISC). **DONE.**
- [x] command parse edges — DONE: `tokenize` quotes/concat/unterminated-EOL/empty-quote/
      comment/blank; all 12 `file-dialog` `FilePurpose` words (+ unknown rejected);
      `set-color` bad-hex vs wrong-length; `convert-palette threshold=` non-numeric / OOR;
      `dock` bad x / y.
- [~] `bake` guards / shore / palette_convert / json. DONE: shore `FixStrength::Mangle`
      converges + region-limited fix stays local; `palette_convert` zero-free-slots plan;
      `json` serializer (`\u00xx` + named-escape precedence, integer-vs-float `write_number`,
      truncated/`non-hex`/unknown/lone-surrogate `\u`). LEFT: `bake` `MAX_BAKED_TILES`
      overflow (needs >65535 distinct composed tiles) and missing-pass (needs a pass-less
      pack) — both impractical to construct in a unit test.
- [x] `statusbar.rs` format test (extracted pure `right_text`, cursor/selection/both/empty);
      `input.rs` `default_keys_all_parse` (every DEFAULT_KEYS action + chord parses). **DONE.**

## Phase 4 — Practices polish (clean wins DONE; large/risky items deferred 2026-06-20)

> The high-confidence items landed (named consts, `pub` narrowing + vestigial
> `Option`, dead-code/`force` removal, the shared `read_multi_header`). The rest
> are the genuine "polish dregs" — each a large/intricate/coupling-adding refactor
> of working, tested code for marginal gain — and are **deferred with rationale**
> rather than churned. Every done step kept clippy/fmt + all suites green.

- [~] `image/multi.rs` RLE decoders. DONE: the byte-identical frame-header parse
      (8-byte dims/hotspots + validation + row-offset table) → shared `read_multi_header`,
      used by both indexed decoders (27 max-assets tests confirm byte-identical decode).
      DEFER the full 4-decoder unification: the rgba decoders receive their header from
      `parse_frames` while the indexed pair parse inline, and the body-vs-shadow RLE
      opcodes differ — a larger restructuring of intricate binary code. **(M)**
- [x] Named consts — worldgen per-step work budgets (`FIELD_CELLS_PER_STEP` /
      `STAMP_ATTEMPTS_PER_STEP` / `FIX_WORK_PER_STEP`). The XOR salts and image_import
      progress weights are single-use and self-documented by inline ASCII comments.
- [x] Narrow over-broad `pub` / vestigial `Option` — `pack.rs` `DIR_*` → `pub(crate)`;
      `user_{tilepacks,maps}_dir`/`stock_templates_dir` `Option<PathBuf>` → `PathBuf`
      (dropped 6 dead None-branches). (`MatchRule` left exported — narrowing cascades
      through the `pub TilePack.matches` field.)
- [x] Dead code / stale comments — removed the vestigial `force` from `Command::New`/
      `Open` (new!/open! are aliases now); fixed `pack.rs`'s stale module doc. Kept by
      design: `SetPass` (retirement shim → pass-paint), `DitherMethod` single variant
      (New-from-Image forward-compat), `slot_rect`'s `allow(dead_code)` (test-only probe).
- [defer] Converge `max-assets` errors onto an `AssetError`/`ImageError` enum. **(M-L)** —
      large error-handling churn across `image/`+`res/` and into the app; the
      `Option`/`Result<_,String>` mix works and is tested.
- [defer] `ini_section.rs` `Box<dyn Any>` TypeId-dispatch → `enum IniEntry`. **(L)** —
      invasive rewrite of the vendored `ini` crate's value storage; the 8 downcasts are
      covered by 45 ini tests.
- [defer] `res/manifest.rs` hand-rolled INI → `ini` crate. **(M)** — would add a
      max-assets→ini cross-crate dependency (against the dep-minimization house rule) to
      replace a ~30-line tested parser; layering-wise an asset crate shouldn't pull in the
      config parser.
- [defer] Toolbox `is_active` predicate. **(M)** — 22-site `const`-button churn + the
      Select-group special-casing (+ a likely-dead brush-size arm), verifiable only by
      headless screenshots, for moderate type-safety gain over a localized, test-guarded
      ladder.
- [defer] `execute` variant→handler table. **(M)** — the backlog itself sanctions relying
      on the P0 `toolbox_commands_route_without_panicking` exhaustiveness test instead; the
      8 `unreachable!` tails are correct routing assertions.
- [defer] Per-frame alloc caching (`picker::items()`, vertex buffers, minimap/units). **(M)**
      — a perf optimization where caching risks staleness bugs, with no perf-regression
      tests and screenshot-only verification; needs profiling to justify the risk.
- [defer] Naming touch-ups (modal cancel button, drag bool, pass suffix, `active_tile`
      field-vs-method shadow, `exec_select` split). **(S)** — cosmetic; the `active_tile`
      shadow is disambiguated by Rust's call parens, so renaming is churn for no functional
      gain.
