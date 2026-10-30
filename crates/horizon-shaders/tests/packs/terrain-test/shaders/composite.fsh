#version 120

uniform sampler2D colortex0;
uniform sampler2D depthtex0;
uniform mat4 gbufferProjectionInverse;
uniform float far;

varying vec2 texcoord;

void main() {
    vec3 color = texture2D(colortex0, texcoord).rgb;
    if (texcoord.x > 0.5) {
        float depth = texture2D(depthtex0, texcoord).r;
        vec4 view = gbufferProjectionInverse * vec4(vec3(texcoord, depth) * 2.0 - 1.0, 1.0);
        color.b = clamp(length(view.xyz / view.w) / far, 0.0, 1.0);
    }
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = vec4(color, 1.0);
}
