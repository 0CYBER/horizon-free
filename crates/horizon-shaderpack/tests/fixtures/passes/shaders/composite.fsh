#version 120
uniform sampler2D colortex3;
varying vec2 texcoord;
#ifdef BLOOM
/*
const int colortex1Format = R8;
*/
#endif
const bool colortex3MipmapEnabled = true; // read with mipmaps
void main() {
#if 0
    /* DRAWBUFFERS:12 */
#endif
    /* RENDERTARGETS: 0,1 */
    gl_FragData[0] = texture2D(colortex3, texcoord);
    gl_FragData[1] = vec4(1.0);
}
