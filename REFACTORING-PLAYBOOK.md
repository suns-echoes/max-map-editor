# Refactoring playbook

How to work on this codebase's structural / cleanup / test work, distilled from
the Phase 0–4 optimization pass (see `OPTIMIZATION-BACKLOG.md` for the *what*;
this is the *how*). Read it before picking up backlog items or any
"clean this up / refactor / add tests" task.

## 1. Behaviour preservation is the bar — and it must be *verified*

A refactor that changes observable behaviour is a bug, not a cleanup. Never
claim "behaviour-preserving" without running the strongest verification the
code has. Match the aggressiveness of the change to the strength of that
verification:

| Area | Strongest check | How aggressive to be |
|---|---|---|
| `map-core` document/codec/worldgen/shore | **equivalence corpus** (23/24 maps byte-identical — only the pre-existing `SNOW_5` differs), **shore ground-truth**, **worldgen golden vectors**, 100+ lib tests | Refactor freely — regressions fail loudly. |
| `map-core` palette/pack/image | unit + the above corpus when palette/compose is touched | Aggressive, but re-run the corpus. |
| app `command`/`state`/`*_io` | 200+ unit tests driving `execute()` + the headless script suite | Confident, but run both. |
| app **GPU draw / panel input / modal layout** | headless render (screenshots) + limited press-simulation only — **no byte-identical check** | Conservative. Pure data/param refactors OK; behavioural/layout rewrites need manual UI testing, so prefer to defer them. |

Run order for a map-core change: `cargo test -p map-core --lib` → the
`equivalence` / `shore_ground_truth` integration tests. For app: `--bins` unit
tests → `cargo test -p max-map-editor --test '*'` (the ~67 s headless render).

## 2. One logical unit per commit; build/clippy/fmt/test green each time

Every commit compiles, passes `cargo clippy` and `cargo fmt --check`, and passes
the relevant suites. Large refactors are done **incrementally** — the 83→55
field `EditorState` de-silo was four commits (one sub-struct each), every one
green and revertible. Don't batch unrelated changes; don't leave the tree red
between commits. Commit messages state *what* + *the verification run*, and end
with the `Co-Authored-By` trailer.

## 3. Deliberate-skip-with-rationale beats both blind-do and silent-skip

Apply engineering judgment, then **write the decision down** (in the commit and
the backlog). Skip or defer when forcing the change would:

- rest on a false premise (the "shared modal metrics" item — the constants are
  per-modal *tuned*, not one repeated value, so unifying changes layouts);
- hurt grep-ability / explicitness (a blanket `Outcome::Failed` wrapper over 141
  varied sites; un-naming the failure-exit constructor);
- reduce safety (removing correct `unreachable!` routing assertions);
- churn working, tested code for marginal gain (per-frame alloc caching, the
  toolbox `is_active` const-churn, the 4-decoder RLE unification);
- add a dependency — including **cross-crate coupling** — against the house rule
  (max-assets→ini for `res/manifest`).

Conversely, *do* the work when it's behaviour-preserving and verifiable, even if
large (the `EditorState` de-silo, `flood4`, `shore` seam dedup). The test is
ROI-and-verification, not size.

## 4. Make the core pure so the shell is testable

The highest-leverage refactor pattern here: split a pure transform from its
IO/rfd/GPU shell, so error and edge paths become unit-testable without fixtures.
Examples that paid off: `TilePack::from_reader` (readers injected → in-memory
loader-error tests), `dialog_default_dir`/`dialog_suggested_name` (rfd-free path
policy), `palette_io`/`settings_io`, `statusbar::right_text`. When you touch a
function that mixes policy + IO, lift the policy out and test it.

## 5. Tooling for safe mechanical edits

- **Node scripts** (never Python) for many-site or tab-sensitive edits. Use
  **word-boundary-guarded** regexes and **assert the replacement count** before
  writing (`console.log` the count; if it's not what you expect, stop). Order
  matters when one field name is a prefix of another (e.g. replace bare
  `.templates` → `.templates.entries` *before* the `.templates_*` variants, or
  rely on `\b`).
- `git mv` for file/module splits (reversible); `project.rs` → `project/` and the
  `grid`/`cellgrid` modules went this way.
- Scratch files go in the project's `temp/`, never `/tmp/`. Tests that touch the
  filesystem use `env!("CARGO_MANIFEST_DIR")` + `temp/<name>` and clean up.

## 6. Trust `cargo build`, not the lagging editor diagnostics

The harness's inline diagnostics lag behind multi-edit sequences and frequently
show *stale* errors (fields "not found" that you already added, dead-code on
just-wired consts). After a batch of edits, **build for ground truth**; don't
chase phantom diagnostics. Rust-analyzer also renders literal control bytes
invisibly — if an `Edit` won't match a string with escapes, the file may contain
real control characters (rewrite the block via a Node script anchored on
control-free text).

## 7. Test the feasible; record the infeasible

Cover the error/edge paths that are practical to construct. When a guard needs an
impractical input (bake's >65535-distinct-tile overflow, the matching
image_import limit), **say so in a comment / the backlog** rather than silently
leaving a gap — "covered everything" should mean it.

## 8. Project specifics worth remembering

- Workspace: app (`max-map-editor`) + `crates/{map-core, max-assets, json, ini}`.
  Rust edition 2024, rust-version 1.85 (pre-trait-upcasting — `Modal` needs
  explicit `as_any`/`into_any`).
- `SNOW_5` equivalence mismatch (2/12544 cells) is **pre-existing and unrelated**
  — it is the *only* corpus map allowed to differ; anything else regressing is on
  you.
- House rules (from the user's global config): minimise dependencies and discuss
  before adding any; prefer JS/TS/Node for tooling; `rustfmt` is the source of
  truth for style; commit/push only when asked, branch first on `main`.
- Keep `OPTIMIZATION-BACKLOG.md` current as you go — tick items, and for every
  deferral leave the rationale inline so the next session doesn't re-litigate it.
