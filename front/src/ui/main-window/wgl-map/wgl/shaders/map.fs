#version 300 es
precision highp float;
precision highp usampler2D;
precision highp usampler2DArray;

// Uniforms
uniform float uZoom;
uniform vec2 uCursor;
uniform float uMapLayer;

// Animation frames for water/effects
uniform uint uAnimationFrame_6fps;
uniform uint uAnimationFrame_8fps;
uniform uint uAnimationFrame_10fps;

// Textures
uniform sampler2D uPaletteTexture;           // RGBA 256x1 palette
uniform highp usampler2DArray uMapTexture;   // RGBA16UI map data (x, y, layer, transform)
uniform highp usampler2DArray uTilesTexture; // R8UI tile atlas

// Input from vertex shader
in vec2 vTexCoord;

// Output color
out vec4 outColor;

// Tile size in pixels
const uint TILE_SIZE = 64u;
const uint TILE_LENGTH = 4096u; // TILE_SIZE * TILE_SIZE

/**
 * Apply tile transformation to texture coordinates.
 * Transformations: N=0, W=1, S=2, E=3, !N=4, !E=5, !S=6, !W=7
 * Bit 2 = horizontal flip, Bits 0-1 = rotation (0=0°, 1=90°CW, 2=180°, 3=270°CW)
 */
vec2 applyTransform(vec2 tc, uint transform) {
	// Center coordinates
	tc -= 0.5;

	// Apply horizontal flip FIRST (bit 2)
	if ((transform & 4u) != 0u) {
		tc.x = -tc.x;
	}

	// Then apply rotation (bits 0-1)
	uint rotation = transform & 3u;
	if (rotation == 1u) {
		// Rotate 90° CW (E)
		tc = vec2(-tc.y, tc.x);
	} else if (rotation == 2u) {
		// Rotate 180° (S)
		tc = vec2(-tc.x, -tc.y);
	} else if (rotation == 3u) {
		// Rotate 270° CW (W)
		tc = vec2(tc.y, -tc.x);
	}

	// Restore coordinates
	return tc + 0.5;
}

void main() {
	// Get map dimensions from texture size
	ivec3 mapSize = textureSize(uMapTexture, 0);
	int mapWidth = mapSize.x;
	int mapHeight = mapSize.y;

	// Calculate which map cell this fragment is in
	// Flip Y because map data has row 0 at top, but OpenGL has Y=0 at bottom
	vec2 mapPos = vec2(vTexCoord.x, 1.0 - vTexCoord.y) * vec2(float(mapWidth), float(mapHeight));
	ivec2 cellCoord = ivec2(floor(mapPos));

	// Bounds check
	if (cellCoord.x < 0 || cellCoord.x >= mapWidth ||
	    cellCoord.y < 0 || cellCoord.y >= mapHeight) {
		discard;
	}

	// Sample map texture to get tile info (x, y, layer, transform)
	// Map stores: R=tileX (row in tile grid), G=tileY (col in tile grid), B=tileLayer, A=transform
	uvec4 mapData = texelFetch(uMapTexture, ivec3(cellCoord, int(uMapLayer)), 0);
	uint tileX = mapData.r;  // Row index in the tile arrangement
	uint tileY = mapData.g;  // Column index in the tile arrangement
	uint tileLayer = mapData.b;
	uint transform = mapData.a;

	// Empty cell check (transform = 255)
	if (transform == 255u) {
		discard;
	}

	// Calculate position within the tile (0-1)
	vec2 tileUV = fract(mapPos);

	// Apply transformation
	tileUV = applyTransform(tileUV, transform);

	// Get tile atlas dimensions (maxTextureSize x maxTextureSize per layer)
	ivec3 atlasSize = textureSize(uTilesTexture, 0);
	uint atlasWidth = uint(atlasSize.x);

	// tilesPerRow = how many tiles fit per texture row
	// With TILE_LENGTH = 4096 and atlasWidth = 4096, tilesPerRow = 1
	// Each texture row contains one tile's worth of data (64*64 = 4096 pixels)
	uint tilesPerRow = atlasWidth / TILE_LENGTH;

	// Calculate pixel position within the tile (0-63)
	uint pixelX = uint(floor(tileUV.x * float(TILE_SIZE)));
	uint pixelY = uint(floor(tileUV.y * float(TILE_SIZE)));

	// Clamp to valid range (prevent artifacts at tile edges)
	pixelX = min(pixelX, TILE_SIZE - 1u);
	pixelY = min(pixelY, TILE_SIZE - 1u);

	// Calculate the tile index in the linear arrangement
	// tileX is the row in tile grid (0 to tilesPerRow-1)
	// tileY is the column in tile grid
	uint tileIndex = tileY * tilesPerRow + tileX;

	// In the atlas texture:
	// - Each texture row contains one tile (when tilesPerRow == 1)
	// - Within a row, pixels are arranged as: tile pixel (tx,ty) at x = ty * 64 + tx
	// So texture coordinate is:
	// - Y = tileIndex (which texture row)
	// - X = pixelY * TILE_SIZE + pixelX (position within the tile's row of data)
	uint atlasPixelX = pixelY * TILE_SIZE + pixelX;
	uint atlasPixelY = tileIndex;

	// Sample the tile atlas to get palette index
	uint paletteIndex = texelFetch(uTilesTexture, ivec3(atlasPixelX, atlasPixelY, tileLayer), 0).r;

	// Index 0 is transparent
	if (paletteIndex == 0u) {
		discard;
	}

	// Look up color from palette
	vec4 color = texelFetch(uPaletteTexture, ivec2(paletteIndex, 0), 0);

	// Highlight cursor cell
	if (cellCoord.x == int(uCursor.x) && cellCoord.y == int(uCursor.y)) {
		color.rgb = mix(color.rgb, vec3(1.0, 1.0, 0.0), 0.3);
	}

	outColor = color;
}
