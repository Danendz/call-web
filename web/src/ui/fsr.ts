import { useEffect, useState, type RefObject } from "react";

// "Sharpen video": the other person's video upscaled to the size it's drawn at with AMD's FidelityFX Super
// Resolution 1 (MIT), as the native app's fsr.rs does. EASU scales it up following the edges, so they stay sharp
// instead of blurring like a plain bilinear stretch, and RCAS sharpens the result a little. Drawn on a canvas over
// the <video>, which keeps playing underneath for its sound, screenshots, and as the picture whenever this can't run.
//
// Only up to 2× (more is left to the browser's own scaling) and only where it's drawn at least 1.125× bigger than it
// came. The upscaled size moves in eighths, so a tile's animation doesn't make a new texture every frame.

/** `?compare=1`: the left half drawn without FSR, as it would be with it off */
const COMPARE = new URLSearchParams(location.search).get("compare") === "1";

/** How RCAS sharpens, in stops down from the strongest; AMD's default. */
const SHARPNESS_STOPS = 0.2;

/** The size to upscale a `width` × `height` frame to, drawn in a `boxW` × `boxH` box (device pixels), if it's worth it. */
export function fsrTarget(width: number, height: number, boxW: number, boxH: number, cover: boolean): [number, number] | null {
  if (!width || !height) return null;
  const sx = boxW / width;
  const sy = boxH / height;
  const scale = Math.floor(Math.min(cover ? Math.max(sx, sy) : Math.min(sx, sy), 2) * 8) / 8;
  return scale >= 1.125 ? [Math.round(width * scale), Math.round(height * scale)] : null;
}

const VERTEX = `#version 300 es
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

const COMMON = `#version 300 es
precision highp float;
precision highp int;
out vec4 outColor;

vec3 load(sampler2D t, ivec2 p) {
  return texelFetch(t, clamp(p, ivec2(0), textureSize(t, 0) - 1), 0).rgb;
}

float luma(vec3 c) {
  return c.b * 0.5 + (c.r * 0.5 + c.g);
}
`;

// Rows count from the top in both textures, as the frame came; only the canvas itself is drawn upside down
const EASU = `${COMMON}
uniform sampler2D src;
uniform vec2 outSize;

// One quadrant's share of the edge direction and length, from the five lumas around it
void easuSet(inout vec2 dir, inout float len, float w, float la, float lb, float lc, float ld, float le) {
  float dirX = ld - lb;
  float lenX = clamp(abs(dirX) / max(max(abs(ld - lc), abs(lc - lb)), 1e-5), 0.0, 1.0);
  float dirY = le - la;
  float lenY = clamp(abs(dirY) / max(max(abs(le - lc), abs(lc - la)), 1e-5), 0.0, 1.0);
  dir += vec2(dirX, dirY) * w;
  len += (lenX * lenX + lenY * lenY) * w;
}

// One of the 12 taps, weighted by a lanczos-like lobe stretched along the edge
void easuTap(inout vec3 color, inout float weight, vec2 off, vec2 dir, vec2 len, float lob, float clp, vec3 c) {
  vec2 v = vec2(dot(off, dir), dot(off, vec2(-dir.y, dir.x))) * len;
  float d2 = min(dot(v, v), clp);
  float wb = 2.0 / 5.0 * d2 - 1.0;
  float wa = lob * d2 - 1.0;
  wb *= wb;
  wa *= wa;
  wb = 25.0 / 16.0 * wb - (25.0 / 16.0 - 1.0);
  float w = wb * wa;
  color += c * w;
  weight += w;
}

void main() {
  vec2 inSize = vec2(textureSize(src, 0));
  vec2 pp = floor(gl_FragCoord.xy) * inSize / outSize + (0.5 * inSize / outSize - 0.5);
  vec2 fp = floor(pp);
  pp -= fp;
  ivec2 p = ivec2(fp);
  //    b c
  //  e f g h
  //  i j k l
  //    n o
  vec3 b = load(src, p + ivec2(0, -1));
  vec3 c = load(src, p + ivec2(1, -1));
  vec3 e = load(src, p + ivec2(-1, 0));
  vec3 f = load(src, p);
  vec3 g = load(src, p + ivec2(1, 0));
  vec3 h = load(src, p + ivec2(2, 0));
  vec3 i = load(src, p + ivec2(-1, 1));
  vec3 j = load(src, p + ivec2(0, 1));
  vec3 k = load(src, p + ivec2(1, 1));
  vec3 l = load(src, p + ivec2(2, 1));
  vec3 n = load(src, p + ivec2(0, 2));
  vec3 o = load(src, p + ivec2(1, 2));
  float bl = luma(b); float cl = luma(c); float el = luma(e); float fl = luma(f); float gl = luma(g); float hl = luma(h);
  float il = luma(i); float jl = luma(j); float kl = luma(k); float ll = luma(l); float nl = luma(n); float ol = luma(o);

  vec2 dir = vec2(0.0);
  float len = 0.0;
  easuSet(dir, len, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
  easuSet(dir, len, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
  easuSet(dir, len, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
  easuSet(dir, len, pp.x * pp.y, gl, jl, kl, ll, ol);

  float dir2 = dot(dir, dir);
  if (dir2 < 1.0 / 32768.0) {
    dir = vec2(1.0, 0.0);
  } else {
    dir *= inversesqrt(dir2);
  }
  len = len * 0.5;
  len *= len;
  float stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
  vec2 len2 = vec2(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
  float lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
  float clp = 1.0 / lob;

  vec3 color = vec3(0.0);
  float weight = 0.0;
  easuTap(color, weight, vec2(0.0, -1.0) - pp, dir, len2, lob, clp, b);
  easuTap(color, weight, vec2(1.0, -1.0) - pp, dir, len2, lob, clp, c);
  easuTap(color, weight, vec2(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
  easuTap(color, weight, vec2(0.0, 1.0) - pp, dir, len2, lob, clp, j);
  easuTap(color, weight, vec2(0.0, 0.0) - pp, dir, len2, lob, clp, f);
  easuTap(color, weight, vec2(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
  easuTap(color, weight, vec2(1.0, 1.0) - pp, dir, len2, lob, clp, k);
  easuTap(color, weight, vec2(2.0, 1.0) - pp, dir, len2, lob, clp, l);
  easuTap(color, weight, vec2(2.0, 0.0) - pp, dir, len2, lob, clp, h);
  easuTap(color, weight, vec2(1.0, 0.0) - pp, dir, len2, lob, clp, g);
  easuTap(color, weight, vec2(1.0, 2.0) - pp, dir, len2, lob, clp, o);
  easuTap(color, weight, vec2(0.0, 2.0) - pp, dir, len2, lob, clp, n);

  // No ringing: kept within the four nearest pixels
  vec3 lo = min(min(f, g), min(j, k));
  vec3 hi = max(max(f, g), max(j, k));
  outColor = vec4(clamp(color / weight, lo, hi), 1.0);
}`;

// RCAS on EASU's output, onto the canvas: sharpens each pixel against its four neighbours as far as they allow
// without clipping
const RCAS = `${COMMON}
uniform sampler2D upscaled;
// The frame as it came, for the half drawn without FSR
uniform sampler2D src;
uniform vec2 outSize;
uniform float sharpness;
uniform bool split;

vec3 rcas(ivec2 p) {
  //    b
  //  d e f
  //    h
  vec3 b = load(upscaled, p + ivec2(0, -1));
  vec3 d = load(upscaled, p + ivec2(-1, 0));
  vec3 e = load(upscaled, p);
  vec3 f = load(upscaled, p + ivec2(1, 0));
  vec3 h = load(upscaled, p + ivec2(0, 1));
  vec3 mn4 = min(min(b, d), min(f, h));
  vec3 mx4 = max(max(b, d), max(f, h));
  vec3 hitMin = mn4 / max(4.0 * mx4, vec3(1e-5));
  vec3 hitMax = (1.0 - mx4) / min(4.0 * mn4 - 4.0, vec3(-1e-5));
  vec3 lobeRgb = max(-hitMin, hitMax);
  float lobe = max(-(0.25 - 1.0 / 16.0), min(max(lobeRgb.r, max(lobeRgb.g, lobeRgb.b)), 0.0)) * sharpness;
  // Less on flat noise, as AMD's RCAS_DENOISE
  float bl = luma(b); float dl = luma(d); float el = luma(e); float fl = luma(f); float hl = luma(h);
  float range = max(max(max(bl, dl), max(fl, hl)), el) - min(min(min(bl, dl), min(fl, hl)), el);
  float noise = clamp(abs(0.25 * (bl + dl + fl + hl) - el) / max(range, 1e-5), 0.0, 1.0);
  lobe *= 1.0 - 0.5 * noise;
  return clamp((lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0), vec3(0.0), vec3(1.0));
}

void main() {
  ivec2 p = ivec2(int(gl_FragCoord.x), int(outSize.y) - 1 - int(gl_FragCoord.y));
  if (split && gl_FragCoord.x < outSize.x * 0.5) {
    // A thin line where the halves meet
    if (gl_FragCoord.x > outSize.x * 0.5 - 2.0) {
      outColor = vec4(1.0);
      return;
    }
    outColor = vec4(texture(src, (vec2(p) + 0.5) / outSize).rgb, 1.0);
    return;
  }
  outColor = vec4(rcas(p), 1.0);
}`;

/** The two passes on one canvas's WebGL2 context. */
class Fsr {
  private readonly easu: WebGLProgram;
  private readonly rcas: WebGLProgram;
  private readonly source: WebGLTexture;
  private readonly upscaled: WebGLTexture;
  private readonly framebuffer: WebGLFramebuffer;
  private size = "";

  constructor(private readonly gl: WebGL2RenderingContext) {
    this.easu = program(gl, EASU);
    this.rcas = program(gl, RCAS);
    this.source = texture(gl, gl.LINEAR);
    this.upscaled = texture(gl, gl.NEAREST);
    this.framebuffer = gl.createFramebuffer()!;
  }

  /** Draws the video's current frame upscaled to `width` × `height`, the canvas's size. */
  draw(video: HTMLVideoElement, width: number, height: number): void {
    const gl = this.gl;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.source);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, video);

    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.upscaled);
    if (this.size !== `${width}x${height}`) {
      this.size = `${width}x${height}`;
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.upscaled, 0);
    }
    gl.viewport(0, 0, width, height);

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.useProgram(this.easu);
    gl.uniform1i(gl.getUniformLocation(this.easu, "src"), 0);
    gl.uniform2f(gl.getUniformLocation(this.easu, "outSize"), width, height);
    gl.drawArrays(gl.TRIANGLES, 0, 3);

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.useProgram(this.rcas);
    gl.uniform1i(gl.getUniformLocation(this.rcas, "upscaled"), 1);
    gl.uniform1i(gl.getUniformLocation(this.rcas, "src"), 0);
    gl.uniform2f(gl.getUniformLocation(this.rcas, "outSize"), width, height);
    gl.uniform1f(gl.getUniformLocation(this.rcas, "sharpness"), 2 ** -SHARPNESS_STOPS);
    gl.uniform1i(gl.getUniformLocation(this.rcas, "split"), COMPARE ? 1 : 0);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  dispose(): void {
    const gl = this.gl;
    gl.deleteProgram(this.easu);
    gl.deleteProgram(this.rcas);
    gl.deleteTexture(this.source);
    gl.deleteTexture(this.upscaled);
    gl.deleteFramebuffer(this.framebuffer);
  }
}

function program(gl: WebGL2RenderingContext, fragment: string): WebGLProgram {
  const shader = (type: number, source: string) => {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, source);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) throw new Error(gl.getShaderInfoLog(s) ?? "");
    return s;
  };
  const p = gl.createProgram()!;
  gl.attachShader(p, shader(gl.VERTEX_SHADER, VERTEX));
  gl.attachShader(p, shader(gl.FRAGMENT_SHADER, fragment));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS) && !gl.isContextLost()) throw new Error(gl.getProgramInfoLog(p) ?? "");
  return p;
}

function texture(gl: WebGL2RenderingContext, filter: number): WebGLTexture {
  const t = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, t);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, filter);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return t;
}

/**
 * While `on`, draws `video` upscaled on `canvas` whenever a new frame comes or its box changes size. True while the
 * canvas holds the picture; false when it isn't worth it or can't run (no WebGL2, context lost), so the video shows.
 */
export function useFsr(video: RefObject<HTMLVideoElement | null>, canvas: RefObject<HTMLCanvasElement | null>, on: boolean): boolean {
  const [active, setActive] = useState(false);
  useEffect(() => {
    const v = video.current;
    const c = canvas.current;
    if (!on || !v || !c) return;
    const gl = c.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false });
    if (!gl) return;
    let fsr: Fsr;
    try {
      fsr = new Fsr(gl);
    } catch (err) {
      console.warn("sharpen video unavailable", err);
      return;
    }
    let lost = false;
    const draw = () => {
      if (lost || v.readyState < v.HAVE_CURRENT_DATA) return;
      const dpr = devicePixelRatio || 1;
      const cover = getComputedStyle(v).objectFit === "cover";
      // The layout box, before any rotation: its sides go with the frame's
      const target = fsrTarget(v.videoWidth, v.videoHeight, v.clientWidth * dpr, v.clientHeight * dpr, cover);
      setActive(!!target);
      if (!target) return;
      const [w, h] = target;
      if (c.width !== w || c.height !== h) {
        c.width = w;
        c.height = h;
      }
      fsr.draw(v, w, h);
    };

    // A new frame: requestVideoFrameCallback where the browser has it, else every screen refresh
    let frame = 0;
    const byFrame = "requestVideoFrameCallback" in v;
    const tick = () => {
      draw();
      frame = byFrame ? v.requestVideoFrameCallback(tick) : requestAnimationFrame(tick);
    };
    tick();
    // Its box or the frame's size changed, or the first frame came before any callback could
    const resized = new ResizeObserver(draw);
    resized.observe(v);
    v.addEventListener("loadeddata", draw);
    v.addEventListener("resize", draw);
    const onLost = () => {
      lost = true;
      setActive(false);
    };
    c.addEventListener("webglcontextlost", onLost);
    return () => {
      if (byFrame) v.cancelVideoFrameCallback(frame);
      else cancelAnimationFrame(frame);
      resized.disconnect();
      v.removeEventListener("loadeddata", draw);
      v.removeEventListener("resize", draw);
      c.removeEventListener("webglcontextlost", onLost);
      fsr.dispose();
      setActive(false);
    };
  }, [video, canvas, on]);
  return active;
}
