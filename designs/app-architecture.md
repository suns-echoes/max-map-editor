# App Architecture Plan

## Recommended Architecture: Feature Slices with Reactive State Modules

This pattern aligns with the existing structure and leverages the reactive library's strengths.

---

## Core Principles

1. **Workspace + Sessions** — A workspace holds multiple project sessions (tabs)
2. **Session-Scoped State** — Each tab owns its own `MapState`, `EditorState`, and `HistoryState`
3. **Feature Slices** — Each major feature gets its own state module + UI components
4. **Actions as Commands** — Mutations happen through action functions
5. **Effects for Synchronization** — Cross-feature side effects using `Effect.on()`
6. **Derived State** — Use `Expr`/`Memo` for computed values

---

## Proposed Structure

```
front/src/
├── state/
│   ├── workspace-state.ts        # Open sessions (tabs) + active tab
│   ├── session-state.ts          # Per-tab aggregate state
│   ├── map-state.ts              # Core map data (per session)
│   ├── editor-state.ts           # Editor mode, active tool, selection (per session)
│   ├── history-state.ts          # Undo/redo stack (per session)
│   └── project-state.ts          # Project metadata, dirty flag (per session)
│
├── features/
│   ├── tile-painting/
│   │   ├── state/
│   │   │   ├── tile-picker.state.ts
│   │   │   ├── pencil.state.ts
│   │   │   └── brush.state.ts
│   │   ├── actions/
│   │   │   ├── pick-tile.action.ts
│   │   │   ├── draw-tile.action.ts
│   │   │   ├── paint-land.action.ts
│   │   │   └── paint-water.action.ts
│   │   ├── algorithms/
│   │   │   └── local-auto-shore.algorithm.ts
│   │   └── ui/
│   │       ├── main-menu/
│   │       │   └── tile-toolbox-toggle-menu-item.component.ts
│   │       ├── toolbox/
│   │       │   ├── tile-toolbox.component.ts
│   │       │   ├── tile-picker-button.component.ts
│   │       │   ├── pencil-button.component.ts
│   │       │   ├── pencil-options.component.ts
│   │       │   ├── brush-button.component.ts
│   │       │   └── brush-options.component.ts
│   │       └── tile-painting.shell.ts
│   │
│   ├── auto-shore/
│   │   ├── actions/
│   │   │   └── auto-shore.action.ts
│   │   ├── algorithms/
│   │   │   └── auto-shore.algorithm.ts
│   │   └── ui/
│   │       └── main-menu/
│   │           └── auto-shore-menu-item.component.ts
│   │
│   ├── image-import/
│   │   ├── actions/
│   │   │   └── image-import.action.ts
│   │   └── ui/
│   │       └── main-menu/
│   │           └── image-import-menu-item.component.ts
│   │
│   ├── pass-editor/
│   │   ├── actions/
│   │   ├── state/
│   │   └── ui/
│   │
│   ├── palette-editor/
│   │   ├── actions/
│   │   ├── state/
│   │   └── ui/
│   │
│   ├── pixel-editor/
│   │   ├── actions/
│   │   ├── state/
│   │   └── ui/
│   │
│   ├── tile-management/
│   │   ├── actions/
│   │   ├── state/
│   │   └── ui/
│   │
│   └── wrl-io/
│       └── actions/
│           ├── wrl-import.action.ts
│           └── wrl-export.action.ts
│
├── actions/    # (existing - keep for app-level actions)
└── ui/         # (existing - keep for layout/components)
```

---

## State Layer Design

### Workspace State (Tabs)

```typescript
// state/workspace-state.ts
import { Value, Expr } from '^reactive';
import type { SessionState } from '^state/session-state.ts';

export type SessionId = string;

export const WorkspaceState = {
	// Global app UI preferences
	theme: new Value<'light' | 'dark'>('dark'),
	layout: new Value<'default' | 'compact'>('default'),

	// All open tabs keyed by session id
	sessions: new Value<Map<SessionId, SessionState>>(new Map()),

	// Active tab
	activeSessionId: new Value<SessionId | null>(null),

	// Derived: active session state
	activeSession: new Expr(() => {
		const id = WorkspaceState.activeSessionId.value;
		const map = WorkspaceState.sessions.value;
		if (!id) return null;
		return map.get(id) ?? null;
	}),
};
```

### Session State (Per-Tab Aggregate)

```typescript
// state/session-state.ts
import type { MapStateType } from '^state/map-state.ts';
import type { EditorStateType } from '^state/editor-state.ts';
import type { HistoryStateType } from '^state/history-state.ts';
import type { ProjectStateType } from '^state/project-state.ts';

export type SessionState = {
	map: MapStateType;
	editor: EditorStateType;
	history: HistoryStateType;
	project: ProjectStateType;
};
```

### Editor State (Per Session)

```typescript
// state/editor-state.ts
import { Value, Expr } from '^reactive';

export type EditorTool = 'select' | 'brush' | 'rect' | 'ellipse' | 'fill' | 'eyedropper';
export type EditorMode = 'tile' | 'pass' | 'pixel';

export type EditorStateType = {
	mode: Value<EditorMode>;
	tool: Value<EditorTool>;
	selectedTile: Value<string | null>;
	brushSize: Value<number>;
	selection: Value<Selection | null>;
	isPainting: Expr<boolean>;
};

export function createEditorState(): EditorStateType {
	const state = {
		mode: new Value<EditorMode>('tile'),
		tool: new Value<EditorTool>('brush'),
		selectedTile: new Value<string | null>(null),
		brushSize: new Value(1),
		selection: new Value<Selection | null>(null),
		isPainting: new Expr(() =>
			state.tool.value !== 'select' &&
			state.selectedTile.value !== null
		),
	};
	return state;
}
```

### History State (Per Session)

```typescript
// state/history-state.ts
import { Value, Expr } from '^reactive';

type HistoryEntry = { type: string; data: unknown; timestamp: number };

export type HistoryStateType = {
	past: Value<HistoryEntry[]>;
	future: Value<HistoryEntry[]>;
	canUndo: Expr<boolean>;
	canRedo: Expr<boolean>;
};

export function createHistoryState(): HistoryStateType {
	const state = {
		past: new Value<HistoryEntry[]>([]),
		future: new Value<HistoryEntry[]>([]),
		canUndo: new Expr(() => state.past.value.length > 0),
		canRedo: new Expr(() => state.future.value.length > 0),
	};
	return state;
}
```

---

## Feature Slice Pattern

> Feature modules should resolve state from the active session (`WorkspaceState.activeSession`) to ensure each tab operates independently.

### Feature State Example

```typescript
// features/tile-painting/state/tile-picker.state.ts
import { Value, Expr } from '^reactive';
import { WorkspaceState } from '^state/workspace-state.ts';

export const TilePickerState = {
	// Pattern mode for flood fill
	pattern: new Value<string[] | null>(null),

	// Recently used tiles
	recentTiles: new Value<string[]>([]),

	// Current tile preview (computed)
	previewTileData: new Expr(() => {
		const session = WorkspaceState.activeSession.value;
		if (!session) return null;
		const tileId = session.editor.selectedTile.value;
		const tiles = session.map.tiles.value;
		if (!tileId || !tiles) return null;
		return tiles.get(tileId)?.data ?? null;
	}),
};
```

### Feature Actions Example

```typescript
// features/tile-painting/actions/draw-tile.action.ts
import { WorkspaceState } from '^state/workspace-state.ts';
import { pushHistory } from '^state/history-state.ts';

export function paintTile(x: number, y: number) {
	const session = WorkspaceState.activeSession.value;
	if (!session) return;
	const tileId = session.editor.selectedTile.value;
	const mapGrid = session.map.map.value;
	if (!tileId || !mapGrid) return;

	// Record for undo
	// `mapGrid` indexing, `width`, and `tileIdToIndex` are provided by MapState helpers
	pushHistory('paint', { x, y, oldTile: mapGrid[...], newTile: tileId });

	// Mutate
	mapGrid[y * width + x] = tileIdToIndex(tileId);
	session.map.map.set(mapGrid); // Trigger reactivity
}

export function floodFill(x: number, y: number) { ... }
export function patternFill(x: number, y: number, pattern: string[]) { ... }
```

---

## Cross-Feature Effects

```typescript
// features/auto-shore/auto-shore.effects.ts
import { Effect } from '^reactive';
import { WorkspaceState } from '^state/workspace-state.ts';
import { autoFixShore } from './auto-shore.algorithm.ts';

// Optionally auto-apply shore after tile painting
export const autoShoreEffect = new Effect(function autoShoreOnWater(self) {
	const session = WorkspaceState.activeSession.value;
	if (!session) return;
	const mode = session.editor.mode.value;
	if (mode !== 'tile') return;

	// Only run when map changes
	const mapGrid = session.map.map.value;
	if (!mapGrid) return;

	// Apply shore algorithm
	autoFixShore(mapGrid);
}, { strong: true }).on([
	WorkspaceState.activeSessionId,
	WorkspaceState.sessions,
]);
```

---

## UI Component Pattern

```typescript
// features/tile-painting/ui/toolbox/tile-toolbox.component.ts
import { Section, Div } from '^reactive/reactive-node.elements.ts';
import { Effect } from '^reactive';
import { WorkspaceState } from '^state/workspace-state.ts';

export function TileToolbox() {
	const container = Section('tile-toolbox').class(style.tileToolbox);

	// Reactive tile list rendering
	new Effect((self) => {
		const session = WorkspaceState.activeSession.value;
		const tiles = session?.map.tiles.value;
		if (!tiles || !session) return;

		container.nodes(
			[...tiles.entries()].map(([id, tile]) =>
				TileThumbnail(id, tile)
					.onClick(() => session.editor.selectedTile.set(id))
			)
		);
	}).on([WorkspaceState.activeSessionId, WorkspaceState.sessions]);

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
| **Independent Tabs** | Each project’s state is isolated and safe from cross-tab mutations |

---

## Planned Features

- [ ] **Tile Painting** — tile picker, tile palette, tile pencil, tile pattern flood fill
- [ ] **Auto Shore** — automatic shore tile placement
- [ ] **Image Import** — import image and convert it into a map
- [ ] **Pass Editor** — edit pass overlay and tile passage types
- [ ] **Palette Editor** — edit palette colors
- [ ] **Pixel Editor** — pixel-level tile editing
- [ ] **Tile Management** — tile cloning, tile dedupe, remove unused tiles from project
- [ ] **WRL Import** — import WRL map file
- [ ] **WRL Export** — export to WRL map file

---

## Suggested Implementation Order

1. **Workspace + Session State** — multi-tab foundation
2. **Editor State** — mode, tool, selection (per-session)
3. **History State** — undo/redo infrastructure (per-session)
4. **Tile Painting** — brush, then fill, then pattern
5. **Tile Picker/Palette UI** — visual tile selection
6. **Auto Shore** — algorithm + optional auto-apply
7. **WRL Import/Export** — file I/O
8. **Image Import** — conversion pipeline
9. **Pass Editor** — separate mode
10. **Palette Editor** — color manipulation
11. **Pixel Editor** — tile-level editing
12. **Tile Management** — clone, dedupe, cleanup
