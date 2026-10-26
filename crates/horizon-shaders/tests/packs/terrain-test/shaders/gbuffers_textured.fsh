#version 120

uniform sampler2D texture;
uniform int renderStage;

varying vec2 texcoord;
varying vec4 glcolor;

void main() {
    vec4 color = texture2D(texture, texcoord) * glcolor;
    // The moon tinted green: the sun and the moon are told apart.
    if (renderStage == MC_RENDER_STAGE_MOON) color.rb *= 0.3;
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
