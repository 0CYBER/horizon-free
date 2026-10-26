#version 120

uniform int renderStage;

varying vec4 glcolor;

void main() {
    vec4 color = glcolor;
    // Stars magenta, the void dark red, the sky disc a little greener: each stage is told apart.
    if (renderStage == MC_RENDER_STAGE_STARS) color.rgb = vec3(1.0, 0.0, 1.0);
    if (renderStage == MC_RENDER_STAGE_VOID) color.rgb = vec3(0.3, 0.0, 0.0);
    if (renderStage == MC_RENDER_STAGE_SKY) color.g = min(color.g + 0.2, 1.0);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
