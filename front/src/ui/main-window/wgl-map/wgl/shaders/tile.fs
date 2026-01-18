#version 300 es
precision highp float;
precision highp usampler2D;

in vec2 vTexCoord;

uniform usampler2D uTileTexture;  // R8UI - palette indices
uniform sampler2D uPaletteTexture; // RGBA palette

out vec4 outColor;

void main() {
	// Get palette index from tile texture (0-255)
	uint paletteIndex = texture(uTileTexture, vTexCoord).r;

	// Look up color from palette
	// Palette is 256x1, so we sample at (index/255, 0.5)
	vec4 color = texture(uPaletteTexture, vec2(float(paletteIndex) / 255.0, 0.5));

	// Index 0 is transparent
	if (paletteIndex == 0u) {
		discard;
	}

	outColor = color;
}
