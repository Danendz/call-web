//! "Sharpen video": the other person's video upscaled to the size it's drawn at with AMD's FidelityFX Super
//! Resolution 1 (MIT), after `gpu.rs` has turned its YUV into RGBA. EASU scales it up following the edges, so they stay
//! sharp instead of blurring like a plain bilinear stretch, and RCAS sharpens the result a little. Worth it when a
//! low bitrate sends less than the screen shows: 720p on a 1440p stage, or a screen share sent at less than 1080p.
//!
//! Only up to 2× (more is left to Slint's own scaling) and only where it's drawn at least 1.125× bigger than it came.
//! The upscaled size moves in eighths, so a tile's animation doesn't make a new texture every frame.

use std::sync::atomic::{AtomicBool, Ordering};

use slint::wgpu_30::wgpu;

/// Settings' "Sharpen video"
pub static SHARPEN: AtomicBool = AtomicBool::new(true);
/// The Debug group's "Compare upscaling": the left half drawn without FSR, as it would be with it off
pub static COMPARE: AtomicBool = AtomicBool::new(false);

/// How RCAS sharpens, in stops down from the strongest; AMD's default.
const SHARPNESS_STOPS: f32 = 0.2;

/// How the video shows on screen, in physical pixels: Slint's image box and whether it fills it (cut to fit) or
/// fits inside it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fit {
    pub width: f32,
    pub height: f32,
    pub cover: bool,
}

/// The size to upscale a `width` × `height` frame to, if it's worth it.
pub fn target(width: u32, height: u32, fit: Fit) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || !SHARPEN.load(Ordering::Relaxed) {
        return None;
    }
    let (sx, sy) = (fit.width / width as f32, fit.height / height as f32);
    let scale = if fit.cover { sx.max(sy) } else { sx.min(sy) };
    let scale = (scale.min(2.0) * 8.0).floor() / 8.0;
    (scale >= 1.125).then(|| ((width as f32 * scale).round() as u32, (height as f32 * scale).round() as u32))
}

/// What the last pass draws: FSR, half and half, or only a bilinear stretch (a saved frame's "before").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Fsr = 0,
    Split = 1,
    Bilinear = 2,
}

impl Mode {
    pub fn current() -> Self {
        if COMPARE.load(Ordering::Relaxed) { Self::Split } else { Self::Fsr }
    }
}

const SHADER: &str = r"
@group(0) @binding(0) var input: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
@group(0) @binding(2) var<uniform> params: Params;
// The frame as it came, for the passes that show it without FSR
@group(0) @binding(3) var source: texture_2d<f32>;

struct Params {
    // RCAS's strength, 1 the strongest
    sharpness: f32,
    // See `Mode`
    mode: u32,
}

struct Out {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

fn load(t: texture_2d<f32>, p: vec2i) -> vec3f {
    let last = vec2i(textureDimensions(t)) - 1;
    return textureLoad(t, clamp(p, vec2i(0), last), 0).rgb;
}

fn luma(c: vec3f) -> f32 {
    return c.b * 0.5 + (c.r * 0.5 + c.g);
}

// EASU: one quadrant's share of the edge direction and length, from the five lumas around it
fn easu_set(dir: ptr<function, vec2f>, len: ptr<function, f32>, w: f32,
            la: f32, lb: f32, lc: f32, ld: f32, le: f32) {
    let dir_x = ld - lb;
    let len_x = clamp(abs(dir_x) / max(max(abs(ld - lc), abs(lc - lb)), 1e-5), 0.0, 1.0);
    let dir_y = le - la;
    let len_y = clamp(abs(dir_y) / max(max(abs(le - lc), abs(lc - la)), 1e-5), 0.0, 1.0);
    *dir += vec2f(dir_x, dir_y) * w;
    *len += (len_x * len_x + len_y * len_y) * w;
}

// EASU: one of the 12 taps, weighted by a lanczos-like lobe stretched along the edge
fn easu_tap(color: ptr<function, vec3f>, weight: ptr<function, f32>, off: vec2f, dir: vec2f, len: vec2f,
            lob: f32, clp: f32, c: vec3f) {
    let v = vec2f(dot(off, dir), dot(off, vec2f(-dir.y, dir.x))) * len;
    let d2 = min(dot(v, v), clp);
    var wb = 2.0 / 5.0 * d2 - 1.0;
    var wa = lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = 25.0 / 16.0 * wb - (25.0 / 16.0 - 1.0);
    let w = wb * wa;
    *color += c * w;
    *weight += w;
}

@fragment fn easu(in: Out) -> @location(0) vec4f {
    let out_size = in.position.xy / in.uv;
    let in_size = vec2f(textureDimensions(input));
    var pp = floor(in.position.xy) * in_size / out_size + (0.5 * in_size / out_size - 0.5);
    let fp = floor(pp);
    pp -= fp;
    let p = vec2i(fp);
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let b = load(input, p + vec2i(0, -1));
    let c = load(input, p + vec2i(1, -1));
    let e = load(input, p + vec2i(-1, 0));
    let f = load(input, p);
    let g = load(input, p + vec2i(1, 0));
    let h = load(input, p + vec2i(2, 0));
    let i = load(input, p + vec2i(-1, 1));
    let j = load(input, p + vec2i(0, 1));
    let k = load(input, p + vec2i(1, 1));
    let l = load(input, p + vec2i(2, 1));
    let n = load(input, p + vec2i(0, 2));
    let o = load(input, p + vec2i(1, 2));
    let bl = luma(b); let cl = luma(c); let el = luma(e); let fl = luma(f); let gl = luma(g); let hl = luma(h);
    let il = luma(i); let jl = luma(j); let kl = luma(k); let ll = luma(l); let nl = luma(n); let ol = luma(o);

    var dir = vec2f(0.0);
    var len = 0.0;
    easu_set(&dir, &len, (1.0 - pp.x) * (1.0 - pp.y), bl, el, fl, gl, jl);
    easu_set(&dir, &len, pp.x * (1.0 - pp.y), cl, fl, gl, hl, kl);
    easu_set(&dir, &len, (1.0 - pp.x) * pp.y, fl, il, jl, kl, nl);
    easu_set(&dir, &len, pp.x * pp.y, gl, jl, kl, ll, ol);

    let dir2 = dot(dir, dir);
    if (dir2 < 1.0 / 32768.0) {
        dir = vec2f(1.0, 0.0);
    } else {
        dir *= inverseSqrt(dir2);
    }
    len = len * 0.5;
    len *= len;
    let stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
    let len2 = vec2f(1.0 + (stretch - 1.0) * len, 1.0 - 0.5 * len);
    let lob = 0.5 + ((1.0 / 4.0 - 0.04) - 0.5) * len;
    let clp = 1.0 / lob;

    var color = vec3f(0.0);
    var weight = 0.0;
    easu_tap(&color, &weight, vec2f(0.0, -1.0) - pp, dir, len2, lob, clp, b);
    easu_tap(&color, &weight, vec2f(1.0, -1.0) - pp, dir, len2, lob, clp, c);
    easu_tap(&color, &weight, vec2f(-1.0, 1.0) - pp, dir, len2, lob, clp, i);
    easu_tap(&color, &weight, vec2f(0.0, 1.0) - pp, dir, len2, lob, clp, j);
    easu_tap(&color, &weight, vec2f(0.0, 0.0) - pp, dir, len2, lob, clp, f);
    easu_tap(&color, &weight, vec2f(-1.0, 0.0) - pp, dir, len2, lob, clp, e);
    easu_tap(&color, &weight, vec2f(1.0, 1.0) - pp, dir, len2, lob, clp, k);
    easu_tap(&color, &weight, vec2f(2.0, 1.0) - pp, dir, len2, lob, clp, l);
    easu_tap(&color, &weight, vec2f(2.0, 0.0) - pp, dir, len2, lob, clp, h);
    easu_tap(&color, &weight, vec2f(1.0, 0.0) - pp, dir, len2, lob, clp, g);
    easu_tap(&color, &weight, vec2f(1.0, 2.0) - pp, dir, len2, lob, clp, o);
    easu_tap(&color, &weight, vec2f(0.0, 2.0) - pp, dir, len2, lob, clp, n);

    // No ringing: kept within the four nearest pixels
    let lo = min(min(f, g), min(j, k));
    let hi = max(max(f, g), max(j, k));
    return vec4f(clamp(color / weight, lo, hi), 1.0);
}

// RCAS, on EASU's output: sharpens each pixel against its four neighbours as far as they allow without clipping
fn rcas(p: vec2i) -> vec3f {
    //    b
    //  d e f
    //    h
    let b = load(input, p + vec2i(0, -1));
    let d = load(input, p + vec2i(-1, 0));
    let e = load(input, p);
    let f = load(input, p + vec2i(1, 0));
    let h = load(input, p + vec2i(0, 1));
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let hit_min = mn4 / max(4.0 * mx4, vec3f(1e-5));
    let hit_max = (1.0 - mx4) / min(4.0 * mn4 - 4.0, vec3f(-1e-5));
    let lobe_rgb = max(-hit_min, hit_max);
    var lobe = max(-(0.25 - 1.0 / 16.0), min(max(lobe_rgb.r, max(lobe_rgb.g, lobe_rgb.b)), 0.0)) * params.sharpness;
    // Less on flat noise, as AMD's RCAS_DENOISE
    let bl = luma(b); let dl = luma(d); let el = luma(e); let fl = luma(f); let hl = luma(h);
    let range = max(max(max(bl, dl), max(fl, hl)), el) - min(min(min(bl, dl), min(fl, hl)), el);
    let noise = clamp(abs(0.25 * (bl + dl + fl + hl) - el) / max(range, 1e-5), 0.0, 1.0);
    lobe *= 1.0 - 0.5 * noise;
    return clamp((lobe * (b + d + f + h) + e) / (4.0 * lobe + 1.0), vec3f(0.0), vec3f(1.0));
}

@fragment fn sharpen(in: Out) -> @location(0) vec4f {
    let p = vec2i(floor(in.position.xy));
    let width = in.position.x / in.uv.x;
    if (params.mode == 2u || (params.mode == 1u && in.position.x < width * 0.5)) {
        // A thin line where the halves meet
        if (params.mode == 1u && in.position.x > width * 0.5 - 2.0) {
            return vec4f(1.0, 1.0, 1.0, 1.0);
        }
        return vec4f(textureSample(source, linear, in.uv).rgb, 1.0);
    }
    return vec4f(rcas(p), 1.0);
}
";

/// The two passes, made once with the converter.
pub struct Fsr {
    layout: wgpu::BindGroupLayout,
    easu: wgpu::RenderPipeline,
    sharpen: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

/// One feed's textures between the passes, made again when its sizes change.
pub struct Upscale {
    device: wgpu::Device,
    /// The frame as it came, in RGBA, which `gpu.rs` converts into
    pub source: wgpu::Texture,
    easu: wgpu::Texture,
    params: wgpu::Buffer,
    easu_bind: wgpu::BindGroup,
    sharpen_bind: wgpu::BindGroup,
}

impl Upscale {
    pub fn target(&self) -> (u32, u32) {
        (self.easu.width(), self.easu.height())
    }

    pub fn fits(&self, device: &wgpu::Device, source: (u32, u32), target: (u32, u32)) -> bool {
        self.device == *device
            && (self.source.width(), self.source.height()) == source
            && (self.easu.width(), self.easu.height()) == target
    }
}

impl Fsr {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fsr"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fsr"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fsr"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |fragment| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fsr"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { easu: pipeline("easu"), sharpen: pipeline("sharpen"), layout, sampler }
    }

    /// Textures for upscaling `source` to `target`; `texture` makes one Slint can draw and the passes can write.
    pub fn upscale(
        &self,
        device: &wgpu::Device,
        source: (u32, u32),
        target: (u32, u32),
        texture: impl Fn(u32, u32) -> wgpu::Texture,
    ) -> Upscale {
        let source = texture(source.0, source.1);
        let easu = texture(target.0, target.1);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fsr params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (easu_bind, sharpen_bind) =
            (self.bind(device, &source, &source, &params), self.bind(device, &easu, &source, &params));
        Upscale { device: device.clone(), source, easu, params, easu_bind, sharpen_bind }
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        input: &wgpu::Texture,
        source: &wgpu::Texture,
        params: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fsr"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view(input)) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&view(source)) },
            ],
        })
    }

    /// Records the passes from `upscale.source`, already converted earlier in `encoder`, into `out`, a texture of the
    /// target size. Sets the mode for every pass recorded since: record one mode per submit.
    pub fn record(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        upscale: &Upscale,
        out: &wgpu::Texture,
        mode: Mode,
    ) {
        let sharpness = (-SHARPNESS_STOPS).exp2();
        let mut params = [0u8; 16];
        params[..4].copy_from_slice(&sharpness.to_le_bytes());
        params[4..8].copy_from_slice(&(mode as u32).to_le_bytes());
        queue.write_buffer(&upscale.params, 0, &params);
        if mode != Mode::Bilinear {
            pass(encoder, &upscale.easu, &self.easu, &upscale.easu_bind);
        }
        pass(encoder, out, &self.sharpen, &upscale.sharpen_bind);
    }
}

fn pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::Texture,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
) {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("fsr"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscales_only_what_is_drawn_bigger_and_at_most_twice() {
        let stage = |width, height| Fit { width, height, cover: false };
        // 720p on a 1440p stage: twice
        assert_eq!(target(1280, 720, stage(2560.0, 1440.0)), Some((2560, 1440)));
        // On a 4K stage still twice; Slint stretches the rest
        assert_eq!(target(1280, 720, stage(3840.0, 2160.0)), Some((2560, 1440)));
        // A 1080p screen on a 1080p stage, or a camera in a small tile that it fills: as it came
        assert_eq!(target(1920, 1080, stage(1920.0, 1080.0)), None);
        assert_eq!(target(1280, 720, Fit { width: 240.0, height: 135.0, cover: true }), None);
        // Fitted inside a tall window by its width, in eighths: 1.6 → 1.5
        assert_eq!(target(1280, 720, stage(2048.0, 2000.0)), Some((1920, 1080)));
        // Cut to fill: by the bigger of the two
        assert_eq!(target(640, 480, Fit { width: 1280.0, height: 600.0, cover: true }), Some((1280, 960)));
    }
}
