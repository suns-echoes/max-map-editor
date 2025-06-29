#version 300 es
precision highp float;
precision highp sampler2DArray;

in vec2 vTexCoord;

uniform float uZoom;
uniform vec2 uCursor;
uniform float uMapLayer;
uniform int uAnimationFrame_6fps;
uniform int uAnimationFrame_8fps;
uniform int uAnimationFrame_10fps;

uniform sampler2D uPaletteTexture;
uniform sampler2DArray uMapTexture;
uniform sampler2D uTilesTexture0;

out vec4 outColor;

void main() {
	// Get map texture 2D size.
	vec2 mapSize = vec2(textureSize(uMapTexture, 0).xy);
	// Get tileSet 2D size. Tile data is 4096x1 pixels.
	vec2 tileDataSize = vec2(4096, 1);
	vec2 tileSetSize = vec2(textureSize(uTilesTexture0, 0)) / tileDataSize;

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
	vec4 tileData = texture(uMapTexture, vec3(cell, uMapLayer)) * 255.0;
	int flags = int(floor(tileData.a));

	// Return transparent color if tile is empty.
	if (flags == 255) {
		outColor = vec4(0.0, 0.0, 0.0, 0.0);
		return;
	}

	// Calculate the tile data offset.
	float x = subCell.x;
	// Check if tile should be flipped.
	if (flags == 4 || flags == 5 || flags == 6 || flags == 7) {
		x = 63.0 - x;
	}
	vec2 tileDataOffset = (tileData.xy + vec2(subCell.y * 64.0 + x, 0.0) / tileDataSize) / tileSetSize;

	// Get the palette index from tile pixel value.
	// The R, G, B, A components of the tile pixel value are used for N, W, S, E transformations.
	float paletteIndex = 0.0;
	if (flags == 0 || flags == 4) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).r * 255.0;
	} else if (flags == 1 || flags == 5) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).g * 255.0;
	} else if (flags == 2 || flags == 6) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).b * 255.0;
	} else if (flags == 3 || flags == 7) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).a * 255.0;
	}

	// Cycle palette colors.
	float rotBy6_6fps = float(uAnimationFrame_6fps % 6);
	float rotBy5_6fps = float(uAnimationFrame_6fps % 5);
	float rotBy7_8fps = float(uAnimationFrame_8fps % 7);
	float rotBy7_10fps = float(uAnimationFrame_10fps % 7);

	// water waves, 7 frames, 8 fps, L-R
	if (paletteIndex >= 96.0 && paletteIndex <= 102.0) {
		paletteIndex -= rotBy7_8fps;
		if (paletteIndex < 96.0) {
			paletteIndex += 7.0;
		}
	}
	// water waves, 7 frames, 8 fps, L-R
	else if (paletteIndex >= 103.0 && paletteIndex <= 109.0) {
		paletteIndex -= rotBy7_8fps;
		if (paletteIndex < 103.0) {
			paletteIndex += 7.0;
		}
	}
	// water waves, 7 frames, 10 fps, L-R
	else if (paletteIndex >= 110.0 && paletteIndex <= 116.0) {
		paletteIndex -= rotBy7_10fps;
		if (paletteIndex < 110.0) {
			paletteIndex += 7.0;
		}
	}
	// water waves, 6 frames, 6 fps, L-R
	else if (paletteIndex >= 117.0 && paletteIndex <= 122.0) {
		paletteIndex -= rotBy6_6fps;
		if (paletteIndex < 117.0) {
			paletteIndex += 6.0;
		}
	// water waves, 5 frames, 6 fps, L-R
	} else if (paletteIndex >= 123.0 && paletteIndex <= 127.0) {
		paletteIndex -= rotBy5_6fps;
		if (paletteIndex < 123.0) {
			paletteIndex += 5.0;
		}
	}

	// Get the color from palette texture.
	vec4 color = texture(uPaletteTexture, vec2(paletteIndex / 255.0, 0.0)).rgba;

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
