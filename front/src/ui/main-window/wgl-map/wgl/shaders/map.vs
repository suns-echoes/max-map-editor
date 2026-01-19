#version 300 es
precision highp float;

// Matrices
uniform mat4 uModel;
uniform mat4 uView;
uniform mat4 uProjection;

// Vertex attributes
layout (location=0) in vec3 aMapPosition;
layout (location=1) in vec2 aTexCoord;

// Outputs to fragment shader
out vec2 vTexCoord;

void main() {
	gl_Position = uProjection * uView * uModel * vec4(aMapPosition.xy * 0.5, 0.0, 1.0);
	vTexCoord = aTexCoord;
}
