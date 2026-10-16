#version 120
varying vec2 texcoord;
/*
const int colortex1Format = RGBA16F;
const bool colortex2Clear = false;
const vec4 colortex3ClearColor = vec4(1.0, 0.0, 0.0, 1.0);
*/
void main() {
    /* DRAWBUFFERS:3 */
    gl_FragData[0] = vec4(texcoord, 0.0, 1.0);
}
