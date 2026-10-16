#version 120
uniform sampler2D colortex0;
varying vec2 texcoord;
void main() {
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = texture2D(colortex0, texcoord);
}
