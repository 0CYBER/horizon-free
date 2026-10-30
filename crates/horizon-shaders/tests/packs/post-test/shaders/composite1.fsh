#version 330 compatibility

uniform sampler2D colortex0;
uniform sampler2D colortex2;
uniform sampler2D noisetex;
uniform float viewWidth;
uniform float viewHeight;

in vec2 texcoord;

/* RENDERTARGETS: 0 */
layout(location = 0) out vec4 outColor;

void main() {
    vec3 color = texture(colortex0, texcoord).rgb;
    if (gl_FragCoord.x > viewWidth * 0.5) {
        float d = clamp(texture(colortex2, texcoord).r, 0.0, 1.0);
        float band = step(0.5, fract(d * 8.0));
        vec3 fog = mix(vec3(0.2, 0.45, 0.9), vec3(0.95, 0.75, 0.3), d);
        color = mix(color, fog, 0.35 + 0.15 * band);
        float grain = texture(noisetex, gl_FragCoord.xy / 256.0).r - 0.5;
        color += grain * 0.04;
    }
    outColor = vec4(color, 1.0);
}
