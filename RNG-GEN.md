# Random Map Generation

Design spec for the editor's random terrain generator (**Tools ▸ Generate Random
Terrain…**, the `generate` command, and `map-core`'s `worldgen`).

A map is built from:

1. one **generator** — the overall land/water layout,
2. a **symmetry** — how the layout is mirrored for fair play,
3. the **common options** — drop zones, obstructions, decorations (shared by every generator),
4. a **shore** method — how coastlines are tiled,
5. a **seed** — makes the result reproducible.

Each generator is **dedicated to a single purpose** and exposes only the properties
that purpose needs.

## Conventions

- **Count** — how many of a thing to place.
- **Min / Max** — a range; each instance's value is chosen at random within it.
- All sizes are in **cells (tiles)**: a blob or patch **radius**, a river **width**
  (tiles across), or an island **distance** (the cell gap between island edges).
- All randomness derives from the **seed**: the same seed + settings always reproduce the same map.

In the property tables below, the second column says what the **Count / value** field
holds (a count, a single value, or — when it doesn't apply); the **Min** and **Max**
columns say what their range measures.

---

## Generators

### 1. Islands

Separate land masses scattered in open water.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Main islands | count | min radius (cells) | max radius (cells) |
| Main islands distance | — | min distance (cells) | max distance (cells) |
| Small islands | count | min radius (cells) | max radius (cells) |
| Small islands distance | — | min distance (cells) | max distance (cells) |
| Rivers | count | min width (cells across) | max width (cells across) |
| Lakes | count | min radius (cells) | max radius (cells) |

- Islands never touch each other or the map edges.
- Favour **varied, irregular shapes** (not uniform blobs).
- The *distance* ranges set the gap between island **edges** — centres are spaced
  `distance + both radii` apart, so the gap holds whatever the island sizes.

### 2. Continents

One or more large land masses surrounded by ocean.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Continents | count | min radius (cells) | max radius (cells) |
| Rivers | count | min width (cells across) | max width (cells across) |
| Lakes | count | min radius (cells) | max radius (cells) |

- Continents do not touch the map edges or each other.

### 3. Central Seas

The inverse of Continents — one or more seas enclosed by land.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Seas | count | min radius (cells) | max radius (cells) |
| Rivers | count | min width (cells across) | max width (cells across) |

- The sea (or seas), surrounded by land, do not touch the map edges or each other.

### 4. Land

A solid landmass, edge to edge, with optional inland water.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Rivers | count | min width (cells across) | max width (cells across) |
| Lakes | count | min radius (cells) | max radius (cells) |

- Plain land; the rivers (curly) and lakes are optional (count 0 = none).

### 5. Rivers

Solid land cut by very curly, meandering river(s).

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Rivers | count | min width (cells across) | max width (cells across) |

- Just land with **very curly** rivers (heavy sine waves, big oxbows, deltas).
- Like every generator's rivers, they enter at a random edge and cross at **any
  angle** (not just horizontal / vertical).

### 6. River Raid

Solid land cut by random, **nearly straight** rivers.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Rivers | count | min width (cells across) | max width (cells across) |

- Just land cut by random, nearly straight rivers (contrast with **Rivers**, which is curly).

### 7. Maze

A navigable labyrinth where **land and water are the main features**: land
corridors you can traverse, separated by water walls. Obstructions (the common
option) are secondary decoration on top of the maze.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Maze | loop count (braid) | min corridor width (cells) | max corridor width (cells) |

- A randomized depth-first maze of land corridors carved into open water; the
  water walls are kept wide enough to coast cleanly.
- **Loop count (braid)** opens that many extra connections, turning the perfect
  maze (all dead-ends) into a more interconnected, looping one.
- The corridor width is picked once per map within the min/max range.

---

## Common options

Shared by every generator.

| Property | Count / value | Min | Max |
| --- | --- | --- | --- |
| Drop zones | count | min radius (cells) | max radius (cells) |
| Obstructions | patches count | min radius (cells) | max radius (cells) |
| Accessibility | value % + mode | — | — |
| Decorations | patches count | min radius (cells) | max radius (cells) |

- **Drop zones** — good starting spots: each **overwrites the terrain with a
  flat, fully-accessible disc of land** of the chosen radius (water included),
  so every landing zone is usable ground; kept **inset** from the map edges
  (never at the very edge) and spread **far apart**.
- **Obstructions** — patches of impassable terrain, of variable size. They may
  sit **right next to shores** at low accessibility (dense maps wall the coast);
  higher accessibility keeps a clear buffer from the water.
- **Accessibility** — a percentage that sets obstruction density (**lower** =
  denser / fewer passages), plus a layout **mode**. The chosen roads / maze are
  planned as a *keep-clear* region **before** obstructions are placed, so feature
  templates land whole and are never broken or partially erased:
  - **random** — patches scattered (as the density alone dictates),
  - **paths** — walkable roads as **multi-step random curves** (each wanders from
    one map extreme through a few random waypoints to another). **One road per 5
    accessibility**, so denser maps still get more passages; the centre is kept
    dense (only a thin spine is cut through it, so it isn't gutted),
  - **labyrinth** — a maze of twisting corridors woven across the whole map
    (dead ends and all), rather than sensible point-to-point paths; the value
    sets how wide / open the corridors are.
- **Decorations** — patches of passable decoration, of variable size.

---

## Symmetry

Mirrors the generated layout for fair-play maps.

- **None** — no mirroring.
- **Left-Right** — mirror across the vertical centre axis (left and right halves match).
- **Top-Bottom** — mirror across the horizontal centre axis (top and bottom halves match).
- **Four Corners** — mirror across both axes (all four quadrants match).
- **Rotate 180 deg** — point symmetry through the centre.

---

## Shore

How coastlines between land and water are tiled.

- **Sweep** — uniform coastline.
- **Loop-walk** — more varied coastline.
- **None** — leave coastlines untiled.

---

## Seed

A number that makes a map reproducible: the same seed + settings always produce the
same map. Leave it empty to roll a fresh seed on every generate.
