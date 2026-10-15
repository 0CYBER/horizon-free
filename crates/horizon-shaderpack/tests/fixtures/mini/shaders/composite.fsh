#version 120
#extension GL_ARB_shader_texture_lod : enable
#include "/lib/common.glsl"
uniform sampler2D colortex0;
varying vec2 texcoord;
void main() {
#if MC_VERSION >= 260300 && defined IS_IRIS
    gl_FragData[0] = texture2D(colortex0, texcoord) * float(MC_RENDER_STAGE_TERRAIN_SOLID);
#endif
}
