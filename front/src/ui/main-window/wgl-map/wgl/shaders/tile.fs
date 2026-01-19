#version 300 es
precision highp float;

in vec2 vTexCoord;

uniform usampler2D uTileTexture;   // R8UI - palette indices
uniform sampler2D uPaletteTexture; // RGBA 256x1
uniform int uTransform;            // 0-7: rotation + flip

out vec4 outColor;

void main() {
	// Apply transformation to texture coordinates
	vec2 tc = vTexCoord;

	// Extract rotation (0-3) and flip (bit 2)
	int rotation = uTransform & 3;
	bool flip = (uTransform & 4) != 0;

	// Center coordinates for rotation
	tc -= 0.5;

	// Apply horizontal flip first
	if (flip) {
		tc.x = -tc.x;
	}

	// Apply rotation (counterclockwise: 0=N, 1=E, 2=S, 3=W)
	if (rotation == 1) {
		tc = vec2(-tc.y, tc.x);
	} else if (rotation == 2) {
		tc = vec2(-tc.x, -tc.y);
	} else if (rotation == 3) {
		tc = vec2(tc.y, -tc.x);
	}

	// Restore coordinates
	tc += 0.5;

	// Sample tile texture to get palette index
	ivec2 texSize = textureSize(uTileTexture, 0);
	ivec2 texel = ivec2(tc * vec2(texSize));
	texel = clamp(texel, ivec2(0), texSize - 1);

	uint paletteIdx = texelFetch(uTileTexture, texel, 0).r;

	// Index 0 is transparent
	if (paletteIdx == 0u) {
		discard;
	}

	// Look up color from palette
	outColor = texelFetch(uPaletteTexture, ivec2(paletteIdx, 0), 0);
}
