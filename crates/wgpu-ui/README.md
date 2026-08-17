# wgpu-ui (vendored)

The first-party `wgpu-ui` toolkit, committed here so this repo builds without a
sibling checkout - a fresh clone, a CI runner or a release build has no other way
to get it (`wgpu-ui` has no public remote).

**This is the copy you edit.** It is what the path dep builds and what CI
compiles. The sibling checkout

    /media/ssd-projects/Rust/wgpu-ui      branch `dev`

is the shared working copy other projects consume, and carries the toolkit's own
test suite. Edit here, then push out and run those tests there:

    node tools/vendor-wgpu-ui.mjs           # push here -> sibling
    node tools/vendor-wgpu-ui.mjs --pull    # take work another agent did there
    node tools/vendor-wgpu-ui.mjs --check   # report anything unsynced (CI, when present)

The sync is **three-way**, not a blind copy: `tools/wgpu-ui-sync.json` holds the
hash of every file as of the last time the two sides agreed, so a push cannot
bury an edit made in the sibling by another project, and a pull cannot revert
one made here. Both sides changed = a conflict you resolve (or `--force`).

`src/`, `assets/` and `rustfmt.toml` are byte-identical on both sides.
`Cargo.toml` and this README are generated - the upstream manifest inherits from
the wgpu-ui workspace, and this copy has to inherit from ours instead. A toolkit
dependency or pin change is the one thing still made upstream, in the sibling's
manifest.

The vendored `rustfmt.toml` is load-bearing: rustfmt resolves the nearest
config to each file, so it keeps `cargo fmt` here reproducing upstream's style
(4 spaces) rather than this repo's (hard tabs, 120 cols), which would rewrite
every line and drown every sync in noise.

`tests/` is deliberately not mirrored: the toolkit's own suite runs in its own
repo, and its golden PNGs are several megabytes. The unit tests inside `src/`
do come along and run with `cargo test`.

The bundled `assets/DejaVuSans.ttf` is `include_bytes!`d by `src/` and ships
under its own license, kept beside it in `assets/DejaVuSans-LICENSE.txt`.
