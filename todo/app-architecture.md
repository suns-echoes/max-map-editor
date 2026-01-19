# App Architecture Plan

## Recommended Architecture: Feature Slices with Reactive State Modules

This pattern aligns with the existing structure and leverages the reactive library's strengths.

---

## Core Principles

1. **Centralized State** — Global `AppState` for shared data (map, tiles, palette)
2. **Feature Slices** — Each major feature gets its own state module + UI components
3. **Actions as Commands** — Mutations happen through action functions
4. **Effects for Synchronization** — Cross-feature side effects using `Effect.on()`
5. **Derived State** — Use `Expr`/`Memo` for computed values

---

## Proposed Structure

```
front/src/
├── state/
│   ├── app-state.ts              # Core map data (existing)
│   ├── editor-state.ts           # Editor mode, active tool, selection
│   ├── history-state.ts          # Undo/redo stack
│   └── project-state.ts          # Project metadata, dirty flag
│
├── features/
│   ├── tile-painting/
│   │   ├── tile-painting.state.ts    # Tool-specific state
│   │   ├── tile-painting.actions.ts  # paint, fill, pattern, etc.
│   │   └── ui/
│   │       ├── tile-picker.component.ts
│   │       ├── tile-palette.component.ts
│   │       └── brush-options.component.ts
│   │
│   ├── auto-shore/
│   │   ├── auto-shore.actions.ts
│   │   └── auto-shore.algorithm.ts
│   │
│   ├── image-import/
│   │   ├── image-import.state.ts
│   │   ├── image-import.actions.ts
│   │   └── ui/
│   │
│   ├── pass-table/
│   │   ├── pass-table.state.ts
│   │   └── ui/
│   │
│   ├── palette-editor/
│   │   ├── palette-editor.state.ts
│   │   ├── palette-editor.actions.ts
│   │   └── ui/
│   │
│   ├── pixel-editor/               # Pixel-level tile editing
│   │   ├── pixel-editor.state.ts
│   │   └── ui/
│   │
│   ├── tile-management/            # Clone, dedupe, cleanup
│   │   ├── tile-management.actions.ts
│   │   └── ui/
│   │
│   └── wrl-io/                     # Import/export WRL
│       ├── wrl-import.action.ts
│       └── wrl-export.action.ts
│
├── actions/                        # (existing - keep for app-level actions)
└── ui/                             # (existing - keep for layout/components)
```

---

## State Layer Design

### Editor State

```typescript
// state/editor-state.ts
import { Value, Expr } from '^reactive';

export type EditorTool = 'select' | 'brush' | 'rect' | 'ellipse' | 'fill' | 'eyedropper';
export type EditorMode = 'ground' | 'water' | 'passTable' | 'pixel';

export const EditorState = {
	mode: new Value<EditorMode>('ground'),
	tool: new Value<EditorTool>('brush'),
	selectedTile: new Value<string | null>(null),
	brushSize: new Value(1),
	selection: new Value<Selection | null>(null),

	// Derived: Is currently painting?
	isPainting: new Expr(() =>
		EditorState.tool.value !== 'select' &&
		EditorState.selectedTile.value !== null
	),
};
```

### History State

```typescript
// state/history-state.ts
import { Value } from '^reactive';

type HistoryEntry = { type: string; data: unknown; timestamp: number };

export const HistoryState = {
	past: new Value<HistoryEntry[]>([]),
	future: new Value<HistoryEntry[]>([]),

	canUndo: new Expr(() => HistoryState.past.value.length > 0),
	canRedo: new Expr(() => HistoryState.future.value.length > 0),
};
```

---

## Feature Slice Pattern

### Feature State Example

```typescript
// features/tile-painting/tile-painting.state.ts
import { Value, Expr } from '^reactive';
import { EditorState } from '^state/editor-state.ts';
import { AppState } from '^state/app-state.ts';

export const TilePaintingState = {
	// Pattern mode for flood fill
	pattern: new Value<string[] | null>(null),

	// Recently used tiles
	recentTiles: new Value<string[]>([]),

	// Current tile preview (computed)
	previewTileData: new Expr(() => {
		const tileId = EditorState.selectedTile.value;
		const tiles = AppState.tiles.value;
		if (!tileId || !tiles) return null;
		return tiles.get(tileId)?.data ?? null;
	}),
};
```

### Feature Actions Example

```typescript
// features/tile-painting/tile-painting.actions.ts
import { AppState } from '^state/app-state.ts';
import { EditorState } from '^state/editor-state.ts';
import { TilePaintingState } from './tile-painting.state.ts';
import { pushHistory } from '^state/history-state.ts';

export function paintTile(x: number, y: number) {
	const tileId = EditorState.selectedTile.value;
	const map = AppState.map.value;
	if (!tileId || !map) return;

	// Record for undo
	pushHistory('paint', { x, y, oldTile: map[...] });

	// Mutate
	map[y * width + x] = tileIdToIndex(tileId);
	AppState.map.set(map); // Trigger reactivity
}

export function floodFill(x: number, y: number) { ... }
export function patternFill(x: number, y: number, pattern: string[]) { ... }
```

---

## Cross-Feature Effects

```typescript
// features/auto-shore/auto-shore.effects.ts
import { Effect } from '^reactive';
import { AppState } from '^state/app-state.ts';
import { EditorState } from '^state/editor-state.ts';
import { autoFixShore } from './auto-shore.algorithm.ts';

// Optionally auto-apply shore after any water tile is painted
export const autoShoreEffect = new Effect(function autoShoreOnWater(self) {
	const mode = EditorState.mode.value;
	if (mode !== 'water') return;

	// Only run when map changes
	const map = AppState.map.value;
	if (!map) return;

	// Apply shore algorithm
	autoFixShore(map);
}, { strong: true }).on([AppState.map, EditorState.mode]);
```

---

## UI Component Pattern

```typescript
// features/tile-painting/ui/tile-palette.component.ts
import { Section, Div } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive';
import { AppState } from '^state/app-state.ts';
import { EditorState } from '^state/editor-state.ts';

export function TilePalette() {
	const container = Section('tile-palette').class(style.tilePalette);

	// Reactive tile list rendering
	new Effect((self) => {
		const tiles = AppState.tiles.value;
		if (!tiles) return;

		container.nodes(
			[...tiles.entries()].map(([id, tile]) =>
				TileThumbnail(id, tile)
					.onClick(() => EditorState.selectedTile.set(id))
			)
		);
	}).on([AppState.tiles]);

	return container;
}
```

---

## Key Benefits

| Pattern | Benefit |
|---------|---------|
| **Feature Slices** | Each feature is self-contained, easy to add/remove |
| **Reactive State** | UI auto-updates, no manual DOM manipulation |
| **Actions** | All mutations in one place, easy to add undo/redo |
| **Effects with `.on()`** | Explicit dependencies, no surprise re-renders |
| **Expr/Memo** | Derived state is always fresh, computed lazily when needed |

---

## Planned Features

- [ ] **Tile Painting** — tile picker, tile palette, tile pencil, tile pattern flood fill
- [ ] **Auto Shore** — automatic shore tile placement
- [ ] **Image Import** — import image and convert it into a map
- [ ] **Pass Table Editor** — edit pass table mode
- [ ] **Palette Editor** — edit palette colors
- [ ] **Pixel Editor** — pixel-level tile editing
- [ ] **Tile Management** — tile cloning, tile dedupe, remove unused tiles from project
- [ ] **WRL Import** — import WRL map file
- [ ] **WRL Export** — export to WRL map file

---

## Suggested Implementation Order

1. **Editor State** — mode, tool, selection (foundation for tools)
2. **History State** — undo/redo infrastructure
3. **Tile Painting** — brush, then fill, then pattern
4. **Tile Picker/Palette UI** — visual tile selection
5. **Auto Shore** — algorithm + optional auto-apply
6. **WRL Import/Export** — file I/O
7. **Image Import** — conversion pipeline
8. **Pass Table Editor** — separate mode
9. **Palette Editor** — color manipulation
10. **Pixel Editor** — tile-level editing
11. **Tile Management** — clone, dedupe, cleanup
