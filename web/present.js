// Puts the console's picture on the canvas the worker owns.
//
// With WebGL2 the picture goes to the GPU straight from the emulator's memory (one texture
// upload and one triangle). `putImageData` on a 2D canvas copies and converts it first, which
// takes several milliseconds a frame on a modest machine. Without WebGL2 the 2D canvas is used.

const VERTEX = `#version 300 es
out vec2 uv;
void main() {
    // One triangle that covers the screen: (0,0) (2,0) (0,2) in texture space.
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    uv = vec2(p.x, 1.0 - p.y);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const FRAGMENT = `#version 300 es
precision mediump float;
uniform sampler2D picture;
in vec2 uv;
out vec4 color;
void main() {
    color = vec4(texture(picture, uv).rgb, 1.0);
}`;

function compile(gl, type, source) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        throw new Error(gl.getShaderInfoLog(shader) || 'shader');
    }
    return shader;
}

/// A presenter that draws with WebGL2, or `null` if the canvas cannot give one.
function createWebGl(canvas) {
    let gl;
    try {
        gl = canvas.getContext('webgl2', {
            alpha: false,
            antialias: false,
            depth: false,
            stencil: false,
            preserveDrawingBuffer: false,
            powerPreference: 'high-performance',
        });
    } catch (_) {
        return null;
    }
    if (!gl) return null;

    let program = null;
    let texture = null;
    let textureWidth = 0;
    let textureHeight = 0;
    let lost = false;

    function setUp() {
        program = gl.createProgram();
        gl.attachShader(program, compile(gl, gl.VERTEX_SHADER, VERTEX));
        gl.attachShader(program, compile(gl, gl.FRAGMENT_SHADER, FRAGMENT));
        gl.linkProgram(program);
        if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
            throw new Error(gl.getProgramInfoLog(program) || 'program');
        }
        gl.useProgram(program);
        gl.uniform1i(gl.getUniformLocation(program, 'picture'), 0);
        texture = gl.createTexture();
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, texture);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        textureWidth = 0;
        textureHeight = 0;
    }

    try {
        setUp();
    } catch (_) {
        return null;
    }
    canvas.addEventListener('webglcontextlost', (event) => {
        event.preventDefault();
        lost = true;
    });
    canvas.addEventListener('webglcontextrestored', () => {
        try {
            setUp();
            lost = false;
        } catch (_) { /* stays lost: the picture stops, the emulation goes on */ }
    });

    return {
        kind: 'webgl2',
        /// Draw `pixels` (RGBA, `width` x `height`).
        draw(pixels, width, height) {
            if (lost) return;
            if (canvas.width !== width || canvas.height !== height) {
                canvas.width = width;
                canvas.height = height;
            }
            gl.viewport(0, 0, width, height);
            if (textureWidth !== width || textureHeight !== height) {
                gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
                textureWidth = width;
                textureHeight = height;
            } else {
                gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
            }
            gl.drawArrays(gl.TRIANGLES, 0, 3);
        },
    };
}

/// A presenter that draws with a 2D canvas.
function create2d(canvas) {
    const context = canvas.getContext('2d');
    return {
        kind: '2d',
        draw(pixels, width, height) {
            if (canvas.width !== width || canvas.height !== height) {
                canvas.width = width;
                canvas.height = height;
            }
            context.putImageData(new ImageData(new Uint8ClampedArray(pixels.buffer, pixels.byteOffset, pixels.length), width, height), 0, 0);
        },
    };
}

/// The best presenter the canvas can give. `prefer2d` forces the 2D canvas (to compare, or
/// to rule WebGL out when something looks wrong).
export function createPresenter(canvas, prefer2d = false) {
    return (!prefer2d && createWebGl(canvas)) || create2d(canvas);
}
