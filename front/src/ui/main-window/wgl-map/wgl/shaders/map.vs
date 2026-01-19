#version 300 es
precision highp float;

// Simple full-screen quad vertex shader
// Vertices are in clip space: (-1,-1) to (1,1)
layout (location=0) in vec2 aPosition;

// Pass screen position to fragment shader (in pixels, centered at 0,0)
out vec2 vScreenPos;

uniform vec2 uScreenSize;  // Canvas width, height

void main() {
	gl_Position = vec4(aPosition, 0.0, 1.0);
	// Convert from clip space [-1,1] to screen pixels centered at origin
	vScreenPos = aPosition * uScreenSize * 0.5;
}
