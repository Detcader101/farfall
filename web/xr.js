// WebXR for FARFALL. The game renders the stereo pair into its own WebGPU
// canvas (left eye | right eye), each eye as a symmetric frustum wide enough
// to hold the headset's asymmetric one; this module owns the XR session, a
// WebGL2 compositor that copies the pair into the headset's framebuffer with
// the true frustum cut out of each half, and the controllers, which drive the
// game's keyboard bindings.

export async function vrSupported() {
  if (!('xr' in navigator)) return false;
  try { return await navigator.xr.isSessionSupported('immersive-vr'); }
  catch { return false; }
}

const VS = `#version 300 es
in vec2 pos; in vec2 uv; out vec2 v;
void main() { v = uv; gl_Position = vec4(pos, 0.0, 1.0); }`;
const FS = `#version 300 es
precision mediump float; in vec2 v; uniform sampler2D img; out vec4 o;
void main() { o = vec4(texture(img, v).rgb, 1.0); }`;

function compile(gl, type, src) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src); gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
  return s;
}

// The asymmetric frustum's tangents from a projection matrix (column-major).
function tangents(m) {
  const l = (1 - m[8]) / m[0], r = (1 + m[8]) / m[0];
  const u = (1 + m[9]) / m[5], d = (1 - m[9]) / m[5];
  return [l, r, u, d];
}

// A synthetic key on the canvas: the game's own bindings do the rest.
function makeKeys(canvas) {
  const down = new Set();
  const send = (code, on) => {
    if (on === down.has(code)) return;
    if (on) down.add(code); else down.delete(code);
    canvas.dispatchEvent(new KeyboardEvent(on ? 'keydown' : 'keyup', { code, key: code, bubbles: true, cancelable: true }));
  };
  return { send, releaseAll() { for (const c of [...down]) send(c, false); } };
}

// xr-standard gamepad: axes 2/3 are the thumbstick, buttons 0 trigger,
// 1 squeeze, 3 stick press, 4/5 the face buttons (A/B right, X/Y left).
function drive(sources, keys) {
  const DEAD = 0.35;
  let lx = 0, ly = 0, rx = 0, ry = 0;
  const b = { lt: 0, rt: 0, lg: 0, rg: 0, a: 0, bb: 0, x: 0, y: 0, ls: 0, rs: 0 };
  for (const src of sources) {
    const gp = src.gamepad; if (!gp) continue;
    const ax = gp.axes.length >= 4 ? [gp.axes[2], gp.axes[3]] : [gp.axes[0] || 0, gp.axes[1] || 0];
    const bt = i => (gp.buttons[i] && (gp.buttons[i].pressed || gp.buttons[i].value > 0.5)) ? 1 : 0;
    if (src.handedness === 'left') { lx = ax[0]; ly = ax[1]; b.lt = bt(0); b.lg = bt(1); b.ls = bt(3); b.x = bt(4); b.y = bt(5); }
    else { rx = ax[0]; ry = ax[1]; b.rt = bt(0); b.rg = bt(1); b.rs = bt(3); b.a = bt(4); b.bb = bt(5); }
  }
  keys.send('KeyD', lx > DEAD); keys.send('KeyA', lx < -DEAD);
  keys.send('KeyW', ly < -DEAD); keys.send('KeyS', ly > DEAD);
  keys.send('ArrowRight', rx > DEAD); keys.send('ArrowLeft', rx < -DEAD);
  keys.send('ArrowDown', ry > DEAD); keys.send('ArrowUp', ry < -DEAD);
  keys.send('ShiftLeft', !!b.lt); keys.send('Space', !!b.rt);
  keys.send('KeyQ', !!b.lg); keys.send('KeyE', !!b.rg);
  keys.send('KeyR', !!b.rs); keys.send('KeyF', !!b.ls);
  keys.send('KeyH', !!b.x); keys.send('KeyV', !!b.y);
  keys.send('KeyX', !!b.a); keys.send('Escape', !!b.bb);
}

export async function enterVR(wasm, canvas) {
  const session = await navigator.xr.requestSession('immersive-vr', { optionalFeatures: ['local'] });
  const glCanvas = document.createElement('canvas');
  const gl = glCanvas.getContext('webgl2', { xrCompatible: true, antialias: false, alpha: false });
  if (!gl) throw new Error('no WebGL2 for the XR compositor');
  await gl.makeXRCompatible();
  const layer = new XRWebGLLayer(session, gl, { antialias: false });
  session.updateRenderState({ baseLayer: layer });
  const space = await session.requestReferenceSpace('local');

  const prog = gl.createProgram();
  gl.attachShader(prog, compile(gl, gl.VERTEX_SHADER, VS));
  gl.attachShader(prog, compile(gl, gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(prog));
  gl.useProgram(prog);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  const posLoc = gl.getAttribLocation(prog, 'pos'), uvLoc = gl.getAttribLocation(prog, 'uv');
  gl.enableVertexAttribArray(posLoc); gl.enableVertexAttribArray(uvLoc);
  gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 16, 0);
  gl.vertexAttribPointer(uvLoc, 2, gl.FLOAT, false, 16, 8);
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.uniform1i(gl.getUniformLocation(prog, 'img'), 0);

  const keys = makeKeys(canvas);
  const views = new Float32Array(22);
  const quad = new Float32Array(16);
  wasm.xr_begin();
  session.addEventListener('end', () => { keys.releaseAll(); wasm.xr_end(); });

  function onFrame(_t, frame) {
    session.requestAnimationFrame(onFrame);
    const pose = frame.getViewerPose(space);
    if (!pose || pose.views.length < 2) return;
    drive(session.inputSources, keys);
    // Both eyes share one target size: the layer's viewport for the first.
    const vp0 = layer.getViewport(pose.views[0]);
    const w = Math.max(1, vp0.width | 0), h = Math.max(1, vp0.height | 0);
    const tans = [];
    pose.views.slice(0, 2).forEach((view, i) => {
      const q = view.transform.orientation, p = view.transform.position;
      const t = tangents(view.projectionMatrix);
      tans.push(t);
      views.set([q.x, q.y, q.z, q.w, p.x, p.y, p.z, t[0], t[1], t[2], t[3]], i * 11);
    });
    if (!wasm.xr_frame(views, w, h)) return;

    // Composite: the pair into the headset's framebuffer, each eye's true
    // frustum read out of its symmetric half.
    gl.bindFramebuffer(gl.FRAMEBUFFER, layer.framebuffer);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, canvas);
    gl.disable(gl.DEPTH_TEST); gl.disable(gl.BLEND);
    pose.views.slice(0, 2).forEach((view, i) => {
      const vp = layer.getViewport(view);
      gl.viewport(vp.x, vp.y, vp.width, vp.height);
      const [l, r, u, d] = tans[i];
      const tx = Math.max(l, r), ty = Math.max(u, d);
      // u across the whole canvas: this eye's half, then the cut inside it.
      const u0 = (i + (tx - l) / (2 * tx)) / 2, u1 = (i + (tx + r) / (2 * tx)) / 2;
      // v from the top of the image (texImage2D of a canvas puts row 0 at the top).
      const vTop = (ty - u) / (2 * ty), vBot = (ty + d) / (2 * ty);
      quad.set([-1, -1, u0, vBot, 1, -1, u1, vBot, -1, 1, u0, vTop, 1, 1, u1, vTop]);
      gl.bufferData(gl.ARRAY_BUFFER, quad, gl.STREAM_DRAW);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    });
  }
  session.requestAnimationFrame(onFrame);
  return session;
}
