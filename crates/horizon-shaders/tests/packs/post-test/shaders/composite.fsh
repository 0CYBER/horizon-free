#version 120

/*
const int colortex2Format = RGBA16F;
*/

uniform sampler2D colortex0;
uniform sampler2D depthtex0;
uniform mat4 gbufferProjectionInverse;
uniform float far;

varying vec2 texcoord;

void main() {
    vec3 color = texture2D(colortex0, texcoord).rgb;
    float depth = texture2D(depthtex0, texcoord).r;
    // Window depth back to a view-space distance.
    vec4 view = gbufferProjectionInverse * vec4(texcoord * 2.0 - 1.0, depth * 2.0 - 1.0, 1.0);
    float distance = -view.z / view.w;
    /* DRAWBUFFERS:02 */
    gl_FragData[0] = vec4(color * vec3(1.08, 1.0, 0.86), 1.0);
    gl_FragData[1] = vec4(distance / far, depth, 0.0, 1.0);
}
