#version 300 es
precision highp float;
precision highp sampler2DArray;
precision highp usampler2DArray;

in vec2 vTexCoord;

uniform float uZoom;
uniform vec2 uCursor;
uniform float uMapLayer;
uniform uint uAnimationFrame_6fps;
uniform uint uAnimationFrame_8fps;
uniform uint uAnimationFrame_10fps;

uniform sampler2D uPaletteTexture;
uniform usampler2DArray uMapTexture;
uniform usampler2DArray uTilesTexture;

out vec4 outColor;

vec3 getMaxCoord(uint x, uint y, uint z) {
	// Get the maximum coordinate for the given tile.
	// This is used to calculate the texture coordinates for the tile.
	return vec3(float(x) / 4096.0, float(y) / 1.0, float(z));
}

void main() {
	// Get map texture 2D size.
	vec2 mapSize = vec2(textureSize(uMapTexture, 0).xy);
	// Get tileSet 2D size. Tile data is 4096x1 pixels.
	vec2 tileDataSize = vec2(4096.0, 1.0);
	vec2 tileSetSize = vec2(textureSize(uTilesTexture, 0)) / tileDataSize;

	// Calculate the cell coordinates.
	vec2 cellXY = floor(vTexCoord * mapSize);
	vec2 cell = cellXY / (mapSize - 1.0);

	// Calculate the sub-cell coordinates.
	vec2 subCell = floor(fract(vTexCoord * mapSize) * 64.0);

	// Get map cell data for tiling:
	//   X = tile X offset in tiles texture
	//   Y = tile Y offset in tiles texture
	//   Z = // TODO: textureUnit of texture containing tile
	//   A = transformation flags values:
	//     0: N (North facing up)
	//     1: W (West facing up)
	//     2: S (South facing up)
	//     3: E (East facing up)
	//     4: !N (X flip, north facing up)
	//     5: !W (X flip, west facing up)
	//     6: !S (X flip, south facing up)
	//     7: !E (X flip, east facing up)
	uvec4 tileDataUI = texture(uMapTexture, vec3(cell, uMapLayer));// * 255.0;
	vec4 tileData = vec4(tileDataUI);// * 256.0;
	uint flags = tileDataUI.a;

	// Return transparent color if tile is empty.
	if (flags == 255u) {
		outColor = vec4(0.0, 0.0, 0.0, 0.0);
		return;
	}

	// Calculate the tile data offset.
	float x = subCell.x;
	// Check if tile should be flipped.
	if (flags == 4u || flags == 5u || flags == 6u || flags == 7u) {
		x = 63.0 - x;
	}
	vec2 tileDataOffset = (tileData.xy + vec2(subCell.y * 64.0 + x, 0.0) / tileDataSize) / tileSetSize;

	// Get the palette index from tile pixel value.
	// The R, G, B, A components of the tile pixel value are used for N, W, S, E transformations.
	uint paletteIndex = 0u;
	vec3 tileSampleCoord = vec3(tileDataOffset, 0.0); // Use layer 0, or replace with correct layer if needed
	uvec4 tileSample = texture(uTilesTexture, tileSampleCoord);

	if (flags == 0u || flags == 4u) {
		paletteIndex = tileSample.r; // * 255.0;
	} else if (flags == 1u || flags == 5u) {
		paletteIndex = tileSample.g; // * 255.0;
	} else if (flags == 2u || flags == 6u) {
		paletteIndex = tileSample.b; // * 255.0;
	} else if (flags == 3u || flags == 7u) {
		paletteIndex = tileSample.a; // * 255.0;
	}

	// Cycle palette colors.
	uint rotBy6_6fps = uAnimationFrame_6fps % 6u;
	uint rotBy5_6fps = uAnimationFrame_6fps % 5u;
	uint rotBy7_8fps = uAnimationFrame_8fps % 7u;
	uint rotBy7_10fps = uAnimationFrame_10fps % 7u;

	// water waves, 7 frames, 8 fps, L-R
	if (paletteIndex >= 96u && paletteIndex <= 102u) {
		paletteIndex -= rotBy7_8fps;
		if (paletteIndex < 96u) {
			paletteIndex += 7u;
		}
	}
	// water waves, 7 frames, 8 fps, L-R
	else if (paletteIndex >= 103u && paletteIndex <= 109u) {
		paletteIndex -= rotBy7_8fps;
		if (paletteIndex < 103u) {
			paletteIndex += 7u;
		}
	}
	// water waves, 7 frames, 10 fps, L-R
	else if (paletteIndex >= 110u && paletteIndex <= 116u) {
		paletteIndex -= rotBy7_10fps;
		if (paletteIndex < 110u) {
			paletteIndex += 7u;
		}
	}
	// water waves, 6 frames, 6 fps, L-R
	else if (paletteIndex >= 117u && paletteIndex <= 122u) {
		paletteIndex -= rotBy6_6fps;
		if (paletteIndex < 117u) {
			paletteIndex += 6u;
		}
	// water waves, 5 frames, 6 fps, L-R
	} else if (paletteIndex >= 123u && paletteIndex <= 127u) {
		paletteIndex -= rotBy5_6fps;
		if (paletteIndex < 123u) {
			paletteIndex += 5u;
		}
	}

	// Get the color from palette texture.
	vec4 color = texture(uPaletteTexture, vec2(float(paletteIndex) / 255.0, 0.0)).rgba;

	// Check if cursor is on cell.
	float cursorFrameWidth = 4.0 * sqrt(2.0) / uZoom;
	if (uCursor == cellXY && (
		subCell.x - cursorFrameWidth < 0.0 ||
		subCell.x + cursorFrameWidth > 63.0 ||
		subCell.y - cursorFrameWidth < 0.0 ||
		subCell.y + cursorFrameWidth > 63.0
	)) {
		float cursorAlpha = sin(float(uAnimationFrame_10fps) / 2.0) / 2.0 + 0.75;
		color = mix(color, color * 2.0, cursorAlpha);
	}

	outColor = color;
}
