#version 300 es
precision highp float;

uniform mat4 uModel;
uniform mat4 uView;
uniform mat4 uProjection;

layout (location=0) in vec4 aPosition;
layout (location=1) in vec2 aTexCoord;
layout (location=2) in float aMapLayer;

out vec2 vTexCoord;
out float vMapLayer;

void main() {
	gl_Position = uProjection * uView * uModel * aPosition;
	vTexCoord = aTexCoord * vec2(1.0, -1.0) + vec2(0.0, 1.0);
	vMapLayer = aMapLayer;
}
