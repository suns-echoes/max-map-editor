# Tileset naming convention

Every tile in an asset pack has a stable **id** of the form

```
<P><F><v><NNN>
└┬┘ └┬┘ └─┬─┘
 │   │    └── NNN  3-digit, zero-padded index within the family (000, 001, …)
 │   └─────── Fv   3-letter family code (see below)
 └─────────── P    pack letter
```

so a full id is **3 letters + 3 digits**, e.g. `GSd004`, `CHa011`, `DPa000`.

The id is the unit of identity across the whole project: it appears in the pack's
`tiles-data.json`, in every map and template that places the tile, and as the key /
reference in the pack sidecars. Renaming a tile means rewriting all of those
together (the editor's **DEV ▸ Edit Match Data** tool and the bake/save paths keep
the sidecars in sync; maps/templates are plain id strings).

## The 3-letter family code `<P><F><v>`

| part | case | meaning |
|------|------|---------|
| **P** — pack letter | UPPER | first letter of the asset-pack id (`CRATER`→`C`, `DESERT`→`D`, `GREEN`→`G`, `SNOW`→`S`, `WATER`→`W`) |
| **F** — feature letter | UPPER | what the tile *is* (see legend) |
| **v** — variant letter | lower | which variant of that feature (`a`, `b`, `c`, …) |

### Feature legend (`F`)

| `F` | feature |
|-----|---------|
| `L` | Land (flat, passable terrain) |
| `S` | Shore (coast / water-edge tiles — the only families with adjacency rules) |
| `M` | Mountain (impassable) |
| `H` | Hill |
| `P` | Pyramid |
| `T` | Tree |
| `C` | Cliff |

Add a new letter here when you introduce a genuinely new feature class, and keep it
consistent across packs.

### Exceptions

- **`WTR` — water** is a universal 3-letter code, *not* `<P><F><v>`. Its tiles are
  `WTR000…` and the pack letter is still `W`. Water is shared by every map (the base
  layer), so it gets one fixed family rather than a per-feature code.
- **`SNOW_DARK` uses UPPERCASE variant letters** (`SSA`, `SLA`, `SCA`, … instead of
  the usual lowercase `SSa`/`SLa`/`SCa`). `SNOW` and `SNOW_DARK` both start with `S`,
  so the uppercase variant is the deliberate way to tell their tiles apart at a glance
  in a map file. SNOW_DARK is otherwise the same families as SNOW, drawn in a darker
  tone.

## Families vs. groups

- **Family** = an id with its trailing digits stripped (`family_of("GSd004") → "GSd"`).
  It is implicit — purely a naming convention, not stored anywhere.
- **Group (variant group)** = an explicit set listed in `tiles.variants.json`. The
  group **name** is almost always identical to the family; the members are
  interchangeable look-variants. The editor resolves a tile to its group with
  `group_of` (variant-group name if any, else the family).

Match rules (`tiles.match.json`) and props (`tiles.props.json`) are keyed by the
**group/family name**, so they apply to every variant at once. The auto-shore engine
groups tiles for matching by `group_of`, which means **DEV ▸ Edit Match Data** can
*link* tiles into a group even across id families — in that one case the group name
differs from the linked tiles' families (it is the authoritative grouping).

## Where each name appears

| file | uses |
|------|------|
| `tiles-data.json` | the ordered id table (`["WTR000", …]`) |
| `tiles.match.json` | keys = group names; entries = `group` / `group:transform` / `__WATER__` / `__LAND__` |
| `tiles.variants.json` | keys = group names; values = member ids |
| `tiles.props.json` | keys = group/family names |
| `tiles.pass.json` | keys = tile ids |
| `tiles.patterns.json` | references tile ids |
| map `*.json` | cells + `tilepass` reference tile ids |
| template `*.json` | cells reference tile ids |

## Current family inventory

| pack | families |
|------|----------|
| CRATER | `CLa` (land) · `CSa…CSm` (shore) · `CMa CMb` (mountain) · `CHa` (hill) |
| DESERT | `DLa DLb DLc` · `DSa…DSm` · `DMa…DMd` · `DPa` (pyramid) |
| GREEN | `GLa GLb GLc` · `GSa…GSo` · `GMa` · `GTa` (tree) |
| SNOW | `SLa SLb SLc` · `SSa…SSl` · `SMa…SMd` · `SCa` (cliff) |
| SNOW_DARK | `SLA SLB SLC` · `SSA…SSL` · `SMA…SME` · `SCA` (uppercase mirror of SNOW) |
| WATER | `WTR` (universal water) |
