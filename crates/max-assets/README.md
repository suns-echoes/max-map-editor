# max-assets

Pure decoders for the original MAX file formats. No rendering, no game logic — bytes in, typed values out. Deliberately free of `wgpu`/`winit` dependencies so the asset extractor, the units compiler, and headless tests can link against it cheaply.

## Modules

- `res` — tagged archive reader (`MAX.RES`).
  - `extract_res_file` — walks the whole archive.
  - `read_res_entry(path, tag)` — pulls a single entry by its string tag.
- `wrl` — tile-indexed map files.
  - `WrlHeader` / `WrlFile`, `read_wrl_header`, `read_wrl_file`.
  - Each WRL carries its own 256-colour palette (for terrain), a bigmap of tile indices, a 64×64 tileset, and a pass-table byte per tile (0=ground, 1=water, 2=shore, 3=obstruction).
- `image` — sprite decoders.
  - `parse_simple_image` — 8-byte header + palette-indexed raster (UI framebits).
  - `parse_big_image` + `image_rle_decode` — RLE-compressed art with an embedded 256-entry palette (portraits, intro art).
  - `parse_multi_image` / `parse_multi_image_all_frames` — N-frame animations, per-row transparency-run encoded (units, buildings).
  - `decode_multi_image_indexed` / `decode_multi_image_shadow_indexed` — versions that keep the frame pixels as palette indices (used by the sprite atlas pipeline, so palette cycling reaches sprites).
  - `IndexedFrame` / `ImageData` — decoded-frame value types. Hot-spot fields are signed (`i32`) because MAX sprites freely anchor above/left of the rectangle (e.g. AWAC's overhead radar dish).
  - `FRAMEPIC_PALETTE_BGRA` — the canonical 256-entry sprite palette (distinct from the WRL per-map palette).
- `units` — `parse_base_unit_data` reads the `D_*.txt` records with frame-strip offsets.
- `color` — small helpers for palette conversion (`indexed_to_color`, `rgb_to_bgra`).

## Hot-spot sign extension

`i16` values on disk are sign-extended to `i32` at the byte-parse site — hot-spots outside the sprite rectangle are common and must not wrap to huge positive values through an accidental `as u32`.
