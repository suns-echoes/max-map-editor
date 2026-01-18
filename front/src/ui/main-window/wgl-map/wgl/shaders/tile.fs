#version 300 es
precision highp float;
precision highp usampler2D;

in vec2 vTexCoord;

uniform usampler2D uTileTexture;  // R8UI - palette indices
uniform sampler2D uPaletteTexture; // RGBA palette
uniform int uTransform;  // 0=none, 1=rot90(E), 2=rot180(S), 3=rot270(W), +4=flipH

out vec4 outColor;

void main() {
	// Apply transformations to texture coordinates
	vec2 tc = vTexCoord;

	// Center coordinates for transformations
	tc -= 0.5;

	// Apply horizontal flip FIRST (bit 2)
	if ((uTransform & 4) != 0) {
		tc.x = -tc.x;
	}

	// Then apply rotation (bits 0-1: rotation)
	int rotation = uTransform & 3;
	if (rotation == 1) {
		// Rotate 90 degrees clockwise (E)
		tc = vec2(-tc.y, tc.x);
	} else if (rotation == 2) {
		// Rotate 180 degrees (S)
		tc = vec2(-tc.x, -tc.y);
	} else if (rotation == 3) {
		// Rotate 270 degrees clockwise (W)
		tc = vec2(tc.y, -tc.x);
	}

	// Restore coordinates
	tc += 0.5;

	// Get palette index from tile texture (0-255)
	uint paletteIndex = texture(uTileTexture, tc).r;

	// Index 0 is transparent
	if (paletteIndex == 0u) {
		discard;
	}

	// Look up color from palette
	// Palette is 256x1, so we sample at (index + 0.5) / 256.0 to center on texel
	vec4 color = texture(uPaletteTexture, vec2((float(paletteIndex) + 0.5) / 256.0, 0.5));

	outColor = color;
}
