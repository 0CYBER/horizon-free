#version 120

uniform sampler2D texture;
uniform sampler2D lightmap;
uniform ivec2 atlasSize;
uniform int renderStage;

varying vec2 texcoord;
varying vec2 lmcoord;
varying vec2 mid;
varying vec4 glcolor;
varying float blockId;

void main() {
    vec4 color = texture2D(texture, texcoord) * glcolor;
    if (color.a < 0.1) discard;
    color.rgb *= texture2D(lightmap, lmcoord).rgb;
    // Less sky light: bluer.
    color.rgb = mix(color.rgb * vec3(0.6, 0.7, 1.2), color.rgb, lmcoord.y);
    // Leaves (block.properties' id 1) yellow.
    if (abs(blockId - 1.0) < 0.5) color.rgb = vec3(color.g, color.g, 0.0);
    // A dot at the face's centre, one texel wide.
    if (length((texcoord - mid) * vec2(atlasSize)) < 1.0) color.rgb = vec3(1.0, 0.0, 1.0);
    // Any other stage than solid terrain (translucent terrain falls back here): red.
    if (renderStage != MC_RENDER_STAGE_TERRAIN_SOLID) color.rgb = vec3(1.0, 0.0, 0.0);
    /* DRAWBUFFERS:0 */
    gl_FragData[0] = color;
}
