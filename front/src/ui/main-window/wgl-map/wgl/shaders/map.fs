#version 300 es
precision highp float;
precision highp usampler2DArray;

// Camera uniforms
uniform vec2 uPan;          // Camera pan in world pixels (map pixels)
uniform float uZoom;        // Zoom factor (pixels per screen pixel)
uniform vec2 uMapSize;      // Map dimensions in cells
uniform int uMapLayer;      // Which map layer to render (0 or 1)
uniform uint uTilesPerRow;  // Pre-computed: atlasWidth / TILE_LENGTH

// Cursor for highlighting
uniform vec2 uCursor;
uniform float uTime;  // Time in seconds for animations

// Textures
uniform sampler2D uPaletteTexture;           // RGBA 256x1 palette
uniform highp usampler2DArray uMapTexture;   // RGBA16UI map data
uniform highp usampler2DArray uTilesTexture; // R8UI tile atlas

// From vertex shader: screen position in pixels, centered at origin
in vec2 vScreenPos;

out vec4 outColor;

// Constants
const float TILE_SIZE = 64.0;
const uint TILE_SIZE_U = 64u;

/**
 * Apply tile transformation to UV coordinates.
 * Bits: [2]=flip, [1:0]=rotation (0=N, 1=W, 2=S, 3=E)
 * N=0°, W=270°CW, S=180°, E=90°CW
 * Order: rotate first, then flip (horizontal)
 */
vec2 applyTransform(vec2 uv, uint t) {
	uv -= 0.5;
	// Rotate first
	uint r = t & 3u;
	if (r == 1u) uv = vec2(uv.y, -uv.x);       // W = 270° CW (90° CCW)
	else if (r == 2u) uv = -uv;                // S = 180°
	else if (r == 3u) uv = vec2(-uv.y, uv.x);  // E = 90° CW
	// Then flip horizontally
	if ((t & 4u) != 0u) uv.x = -uv.x;
	return uv + 0.5;
}

/**
 * Check if a pixel is on the cursor frame edge.
 * frameUV is the frame width in UV space (0-1).
 */
bool isOnCursorFrame(vec2 tileUV, float frameUV) {
	return tileUV.x < frameUV || tileUV.x > (1.0 - frameUV) ||
	       tileUV.y < frameUV || tileUV.y > (1.0 - frameUV);
}

void main() {
	// Convert screen position to world position (map pixels)
	// Y is flipped: screen Y up = world Y down (map row 0 at top)
	vec2 worldPos = vec2(
		vScreenPos.x / uZoom + uPan.x + uMapSize.x * TILE_SIZE * 0.5,
		-vScreenPos.y / uZoom + uPan.y + uMapSize.y * TILE_SIZE * 0.5
	);

	// Convert world pixels to cell coordinates
	vec2 cellPosF = worldPos / TILE_SIZE;
	ivec2 cell = ivec2(floor(cellPosF));

	// Bounds check
	if (cell.x < 0 || cell.x >= int(uMapSize.x) ||
	    cell.y < 0 || cell.y >= int(uMapSize.y)) {
		discard;
	}

	// Fetch map data: R=tileX, G=tileY, B=tileLayer, A=transform
	uvec4 mapData = texelFetch(uMapTexture, ivec3(cell, uMapLayer), 0);

	// Empty cell (transform=255)
	if (mapData.a == 255u) {
		discard;
	}

	// Position within tile (0-1)
	vec2 tileUV = fract(cellPosF);
	tileUV = applyTransform(tileUV, mapData.a);

	// Pixel within tile (0-63)
	uint px = min(uint(tileUV.x * TILE_SIZE), TILE_SIZE_U - 1u);
	uint py = min(uint(tileUV.y * TILE_SIZE), TILE_SIZE_U - 1u);

	// Tile index in atlas
	uint tileIndex = mapData.g * uTilesPerRow + mapData.r;

	// Sample tile atlas: X = py*64+px, Y = tileIndex, Z = tileLayer
	uint paletteIdx = texelFetch(uTilesTexture, ivec3(py * TILE_SIZE_U + px, tileIndex, mapData.b), 0).r;

	// Transparent
	if (paletteIdx == 0u) {
		discard;
	}

	// Get color from palette
	outColor = texelFetch(uPaletteTexture, ivec2(paletteIdx, 0), 0);

	// Cursor frame highlight with fade in/out (additive blending)
	// Frame is always 2 screen pixels thick, converted to UV space
	if (cell.x == int(uCursor.x) && cell.y == int(uCursor.y)) {
		float screenPixels = 2.0;
		float frameUV = screenPixels / uZoom / TILE_SIZE;
		vec2 cellUV = fract(cellPosF);  // Use original UV before transform
		if (isOnCursorFrame(cellUV, frameUV)) {
			// Slow blink: 1.5s period, fade between 0.3 and 0.8 intensity
			float blink = 0.55 + 0.25 * sin(uTime * 4.18879);  // 2*PI / 1.5 ≈ 4.189
			// Additive blend: add yellow glow
			outColor.rgb += vec3(1.0, 1.0, 0.3) * blink;
		}
	}
}
