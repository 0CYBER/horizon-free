#version 120

attribute vec4 mc_midTexCoord;
attribute vec4 mc_Entity;

varying vec2 texcoord;
varying vec2 lmcoord;
varying vec2 mid;
varying vec4 glcolor;
varying float blockId;

void main() {
    gl_Position = ftransform();
    texcoord = (gl_TextureMatrix[0] * gl_MultiTexCoord0).xy;
    lmcoord = (gl_TextureMatrix[1] * gl_MultiTexCoord1).xy;
    mid = mc_midTexCoord.xy;
    glcolor = gl_Color;
    blockId = mc_Entity.x;
}
