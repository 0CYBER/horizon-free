#version 120

uniform sampler2D colortex0;

varying vec2 texcoord;

void main() {
    vec3 color = texture2D(colortex0, texcoord).rgb;
    vec2 centred = texcoord - 0.5;
    float vignette = 1.0 - dot(centred, centred) * 0.9;
    gl_FragColor = vec4(color * vignette, 1.0);
}
