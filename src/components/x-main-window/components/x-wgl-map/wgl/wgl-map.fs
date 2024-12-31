#version 300 es
precision highp float;
precision highp sampler2DArray;

in vec2 vTexCoord;

uniform vec2 uCursor;
uniform float uMapLayer;

uniform sampler2D uPaletteTexture;
uniform sampler2DArray uMapTexture;
uniform sampler2D uTilesTexture0;

out vec4 outColor;

void main() {
	// if (uMapLayer == 1.0) {
	// 	outColor = vec4(1.0, 1.0, 0.0, 1.0);
	// 	return;
	// }
	// if (uMapLayer == 0.0) {
	// 	outColor = vec4(0.0, 1.0, 0.0, 1.0);
	// 	return;
	// }

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

	// Check if cursor is on cell.
	if (uCursor == cellXY && (subCell.x < 2.0 || subCell.x > 61.0 || subCell.y < 2.0 || subCell.y > 61.0)) {
		outColor = vec4(1.0, 1.0, 1.0, 1.0);
		return;
	}

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
		paletteIndex = texture(uTilesTexture0, tileDataOffset).r;
	} else if (flags == 1 || flags == 5) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).g;
	} else if (flags == 2 || flags == 6) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).b;
	} else if (flags == 3 || flags == 7) {
		paletteIndex = texture(uTilesTexture0, tileDataOffset).a;
	}
	// Get the color from palette texture.
	vec4 color = texture(uPaletteTexture, vec2(paletteIndex, 0.0));

	outColor = color;
}
