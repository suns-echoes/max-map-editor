# Production Code Review

## ❌ Critical Issues

1. Memory Leaks - Missing cleanup
  - Event listeners on window are never removed (lines 260-277)
  - ResizeObserver is never disconnected
  - animationFrameId is declared but cancelAnimationFrame is never called
  - WebGL resources (textures, buffers, programs) are never disposed

2. Non-null assertions (!) hide potential runtime errors
  - wgl-map.component.ts:203: canvasElement.parentElement!
  - wgl-renderer.ts:50-67: Multiple ! on uniform locations

3. No error handling for async operations
  - wgl-map.component.ts:423-425: IIFE with await has no .catch()

## ⚠️ Medium Issues

4. setTimeout for DOM readiness is fragile
  - Using setTimeout(() => {...}, 0) (line 201) is unreliable. Should use a mount lifecycle hook or MutationObserver.

5. Unused variable
  - animationFrameId is declared but never used to cancel animation (line 352)

6. Console.log statements in production
  - Multiple console.log calls in wgl-renderer.ts (lines 119, 219, 234, 371)

7. Hardcoded values
  - Color cycle data is hardcoded (lines 334-346)
  - Map path is hardcoded: 'resources/maps/GREEN_1.json'

8. Magic numbers without constants
  - 0.5, 0.2, 0.01, 0.9, 1.1 in various places

9. Missing TypeScript strict typing
  - MapProject type used without import (line 30)

## 💡 Improvements

10. Allocating Float32Array every frame
  - drawRect and drawTileById allocate new Float32Array per call - should reuse a single buffer

11. Debug panel in production
  - Debug info Pre element should be conditional based on environment

12. No component unmount handling
  - Effects created with new Effect() should be disposed when component unmounts

---
Would you like me to fix any of these issues? I'd recommend starting with:

Resource cleanup (memory leaks)
Remove console.log statements
Add error handling
