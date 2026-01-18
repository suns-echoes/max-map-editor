#version 300 es
precision highp float;

uniform mat4 uProjection;

layout (location=0) in vec2 aPosition;

void main() {
	gl_Position = uProjection * vec4(aPosition, 0.0, 1.0);
}
