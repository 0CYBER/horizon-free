#version 120

varying vec4 glcolor;

void main() {
    gl_Position = ftransform();
    glcolor = gl_Color;
}
