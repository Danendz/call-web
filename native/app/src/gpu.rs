//! Video converted to RGBA by the GPU when the window draws with wgpu. Each frame's Y, U and V planes go up as they
//! are, 1.5 bytes a pixel instead of RGBA's 4, and a shader turns them into a texture Slint draws as an image: no
//! per-pixel work on the CPU. Runs on the video threads, on the device and queue Slint draws with; Skia submits on
//! that same queue, so a texture is written only after the frames that drew it before. The conversion is submitted
//! from the UI thread, though: before resizing the window's surface wgpu waits for the queue to empty, and a submit
//! from another thread during that wait is an error that panics (maximizing the window crashed the app).
//!
//! On Vulkan (Android) every submit goes through the UI thread, not only the conversion's: Skia submits to Vulkan's raw
//! queue from there without wgpu's lock, and Vulkan needs every submit to one queue synchronized. Uploads and
//! recording stay on the video threads; wgpu only reaches the queue when it submits.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use engine::{Chroma, Planes as Frame};

use crate::fsr::{Fsr, Mode, Upscale};
use slint::wgpu_30::wgpu;
use slint::{Rgba8Pixel, SharedPixelBuffer};

static CONVERTER: Mutex<Option<Arc<Converter>>> = Mutex::new(None);

/// Which converter, and so which of Slint's devices, a texture was made on. Not `wgpu::Device`'s `==`, which compares
/// ids that each wgpu instance numbers from the start: on Android Slint makes a new instance for each new surface, so
/// a device since torn down can equal the one drawing now, and its textures crash it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DeviceKey(u64);

static NEXT_KEY: AtomicU64 = AtomicU64::new(0);

/// Called once the window draws with wgpu; until then, and with OpenGL, video is converted on the CPU.
/// `CALL_CPU_VIDEO=1` keeps it there, to compare the two.
pub fn start(device: &wgpu::Device, queue: &wgpu::Queue) {
    let backend = device.adapter_info().backend;
    let converter = (matches!(backend, wgpu::Backend::Metal | wgpu::Backend::Dx12 | wgpu::Backend::Vulkan)
        && std::env::var_os("CALL_CPU_VIDEO").is_none())
    .then(|| Arc::new(Converter::new(device.clone(), queue.clone())));
    if let Ok(mut current) = CONVERTER.lock() {
        *current = converter;
    }
}

/// Called when the window's renderer is torn down: its device goes with it, so video goes back to the CPU.
pub fn stop() {
    if let Ok(mut current) = CONVERTER.lock() {
        *current = None;
    }
}

pub fn converter() -> Option<Arc<Converter>> {
    CONVERTER.lock().ok()?.clone()
}

pub struct Converter {
    key: DeviceKey,
    device: wgpu::Device,
    queue: wgpu::Queue,
    i420: Pipeline,
    nv12: Pipeline,
    sampler: wgpu::Sampler,
    fsr: Fsr,
    /// Vulkan: submits only from the UI thread, see the top of this file
    ui_submits: bool,
}

struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// One feed's plane textures, written again by every frame of the same size and format on the same device.
pub struct Planes {
    key: DeviceKey,
    size: (u32, u32),
    nv12: bool,
    /// Y, then U and V or, for NV12, UV
    textures: Vec<wgpu::Texture>,
    bind_group: wgpu::BindGroup,
}

const SHADER: &str = r"
@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var u_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;
@group(0) @binding(3) var planes: sampler;

struct Out {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

// One triangle covering the whole target
@vertex fn vertex(@builtin(vertex_index) i: u32) -> Out {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return Out(vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

// BT.601 limited range, as libyuv's I420 to RGBA, so the picture looks as it did converted on the CPU
fn rgba(y: f32, u: f32, v: f32) -> vec4f {
    let l = (y - 16.0 / 255.0) * 1.164;
    let rgb = vec3f(l + 1.596 * (v - 0.5), l - 0.391 * (u - 0.5) - 0.813 * (v - 0.5), l + 2.018 * (u - 0.5));
    return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);
}

@fragment fn i420(in: Out) -> @location(0) vec4f {
    let y = textureSample(y_plane, planes, in.uv).r;
    return rgba(y, textureSample(u_plane, planes, in.uv).r, textureSample(v_plane, planes, in.uv).r);
}

// U and V interleaved in the second plane, as its red and green
@fragment fn nv12(in: Out) -> @location(0) vec4f {
    let uv = textureSample(u_plane, planes, in.uv).rg;
    return rgba(textureSample(y_plane, planes, in.uv).r, uv.x, uv.y);
}
";

impl Converter {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv to rgba"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler = wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        // NV12 has no binding 2: its U and V share binding 1
        let pipeline = |fragment, entries: &[wgpu::BindGroupLayoutEntry]| {
            let layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(fragment), entries });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(fragment),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
            });
            Pipeline { pipeline, layout }
        };
        let i420 = pipeline("i420", &[plane(0), plane(1), plane(2), sampler]);
        let nv12 = pipeline("nv12", &[plane(0), plane(1), sampler]);
        // Linear, so the half-size chroma is smoothed rather than blocky; Y is sampled at its texel centers
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video planes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fsr = Fsr::new(&device);
        let ui_submits = device.adapter_info().backend == wgpu::Backend::Vulkan;
        let key = DeviceKey(NEXT_KEY.fetch_add(1, Ordering::Relaxed));
        Self { key, device, queue, i420, nv12, sampler, fsr, ui_submits }
    }

    pub fn key(&self) -> DeviceKey {
        self.key
    }

    /// A texture for converted frames, in the format and with the uses Slint takes as an image.
    pub fn texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video frame"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    /// Uploads `frame` and records its conversion into `out`, a [`Self::texture`] of the frame's size or of a bigger
    /// one to upscale it to (`fsr.rs`, through `upscale`), for [`Self::submit`]. A frame whose planes are shorter
    /// than its size says is skipped (None): wgpu treats a short write as a bug and panics.
    pub fn convert(
        &self,
        planes: &mut Option<Planes>,
        upscale: &mut Option<Upscale>,
        frame: &Frame<'_>,
        out: &wgpu::Texture,
    ) -> Option<wgpu::CommandBuffer> {
        let (width, height) = (frame.width, frame.height);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let fits = |data: &[u8], stride: u32, row: u32, h: u32| {
            stride >= row && (stride as usize * (h as usize - 1) + row as usize) <= data.len()
        };
        let chroma_fits = match frame.chroma {
            Chroma::I420 { u, v, u_stride, v_stride } => {
                fits(u, u_stride, chroma_width, chroma_height) && fits(v, v_stride, chroma_width, chroma_height)
            }
            Chroma::Nv12 { uv, stride } => fits(uv, stride, chroma_width * 2, chroma_height),
        };
        let target = (out.width(), out.height());
        let upscaled = target != (width, height);
        if width == 0
            || height == 0
            || (upscaled && (target.0 < width || target.1 < height))
            || !fits(frame.y, frame.y_stride, width, height)
            || !chroma_fits
        {
            return None;
        }
        let nv12 = matches!(frame.chroma, Chroma::Nv12 { .. });
        let planes = match planes {
            Some(planes) if planes.size == (width, height) && planes.nv12 == nv12 && planes.key == self.key => planes,
            _ => planes.insert(self.planes(width, height, nv12)),
        };
        let data = match frame.chroma {
            Chroma::I420 { u, v, u_stride, v_stride } => vec![(frame.y, frame.y_stride), (u, u_stride), (v, v_stride)],
            Chroma::Nv12 { uv, stride } => vec![(frame.y, frame.y_stride), (uv, stride)],
        };
        for (texture, (data, stride)) in planes.textures.iter().zip(data) {
            self.write(texture, data, stride);
        }

        let upscale = if upscaled {
            if !upscale.as_ref().is_some_and(|u| u.fits(self.key, (width, height), target)) {
                *upscale =
                    Some(self.fsr.upscale(&self.device, self.key, (width, height), target, |w, h| self.texture(w, h)));
            }
            upscale.as_ref()
        } else {
            *upscale = None;
            None
        };
        let view = upscale.map_or(out, |u| &u.source).create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yuv to rgba"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&if nv12 { &self.nv12 } else { &self.i420 }.pipeline);
            pass.set_bind_group(0, &planes.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(upscale) = upscale {
            self.fsr.record(&self.queue, &mut encoder, upscale, out, Mode::current());
        }
        Some(encoder.finish())
    }

    /// Runs a conversion from [`Self::convert`]. On the UI thread only, which resizes the window's surface.
    pub fn submit(&self, commands: wgpu::CommandBuffer) {
        self.queue.submit(Some(commands));
    }

    /// Submits from a thread that isn't the UI's, for work it then waits for. On Vulkan the UI thread submits it and
    /// this waits for that; None if the app is closing and the UI thread no longer runs anything.
    fn submit_off_ui(&self, commands: wgpu::CommandBuffer) -> Option<wgpu::SubmissionIndex> {
        if !self.ui_submits {
            return Some(self.queue.submit(Some(commands)));
        }
        let (tx, rx) = mpsc::channel();
        let queue = self.queue.clone();
        slint::invoke_from_event_loop(move || {
            let _ = tx.send(queue.submit(Some(commands)));
        })
        .ok()?;
        rx.recv().ok()
    }

    fn planes(&self, width: u32, height: u32, nv12: bool) -> Planes {
        let plane = |w, h, format| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video plane"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let y = plane(width, height, wgpu::TextureFormat::R8Unorm);
        let textures = if nv12 {
            vec![y, plane(chroma_width, chroma_height, wgpu::TextureFormat::Rg8Unorm)]
        } else {
            let chroma = || plane(chroma_width, chroma_height, wgpu::TextureFormat::R8Unorm);
            vec![y, chroma(), chroma()]
        };
        let views: Vec<_> = textures.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        let mut entries: Vec<_> = (0..)
            .zip(&views)
            .map(|(binding, view)| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) })
            .collect();
        entries.push(wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video planes"),
            layout: &if nv12 { &self.nv12 } else { &self.i420 }.layout,
            entries: &entries,
        });
        Planes { key: self.key, size: (width, height), nv12, textures, bind_group }
    }

    fn write(&self, texture: &wgpu::Texture, data: &[u8], stride: u32) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None },
            texture.size(),
        );
    }

    /// The Debug group's "Save frame": a feed's last frame as it came, stretched to the upscaled size without FSR, and
    /// with it. Waits for the GPU, so keep it off the UI thread.
    #[cfg(not(target_os = "android"))]
    pub fn snapshot(&self, upscale: &Upscale) -> Option<[SharedPixelBuffer<Rgba8Pixel>; 3]> {
        if !upscale.fits(self.key, (upscale.source.width(), upscale.source.height()), upscale.target()) {
            return None;
        }
        let (width, height) = upscale.target();
        let (before, after) = (self.texture(width, height), self.texture(width, height));
        for (out, mode) in [(&before, Mode::Bilinear), (&after, Mode::Fsr)] {
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            self.fsr.record(&self.queue, &mut encoder, upscale, out, mode);
            self.submit_off_ui(encoder.finish())?;
        }
        Some([self.read(&upscale.source)?, self.read(&before)?, self.read(&after)?])
    }

    /// A converted frame's pixels, for a screenshot. Waits for the GPU, so keep it off the UI thread.
    pub fn read(&self, texture: &wgpu::Texture) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
        let (width, height) = (texture.width(), texture.height());
        let row = width * 4;
        // Copies to a buffer go in rows padded to 256 bytes
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: None },
            },
            texture.size(),
        );
        let submitted = self.submit_off_ui(encoder.finish())?;
        let (tx, rx) = mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device.poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None }).ok()?;
        rx.recv().ok()?.ok()?;
        let mapped = buffer.slice(..).get_mapped_range().ok()?;
        let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
        for (dst, src) in pixels.make_mut_bytes().chunks_exact_mut(row as usize).zip(mapped.chunks(padded as usize)) {
            dst.copy_from_slice(src.get(..row as usize)?);
        }
        Some(pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// wgpu's native futures are ready when made, so one poll does.
    fn now<T>(future: impl Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("wgpu's future wasn't ready"),
        }
    }

    fn converter() -> Option<Converter> {
        let adapter = now(wgpu::Instance::default().request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = now(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Converter::new(device, queue))
    }

    fn green(pixels: &SharedPixelBuffer<Rgba8Pixel>, x: u32, y: u32) -> u8 {
        pixels.as_slice()[(y * pixels.width() + x) as usize].g
    }

    /// The biggest step between neighbouring pixels along a row: how sharp the edge in it is.
    fn steepest(pixels: &SharedPixelBuffer<Rgba8Pixel>, y: u32) -> u8 {
        (1..pixels.width()).map(|x| green(pixels, x, y).abs_diff(green(pixels, x - 1, y))).max().unwrap()
    }

    #[test]
    fn fsr_upscales_with_a_sharper_edge_than_a_stretch() {
        let Some(gpu) = converter() else {
            eprintln!("no GPU here, skipped");
            return;
        };
        // Dark on the left, bright on the right, the edge between pixels 30 and 31
        let (width, height) = (64u32, 36u32);
        let y: Vec<u8> = (0..height).flat_map(|_| (0..width).map(|x| if x < 31 { 40 } else { 200 })).collect();
        let chroma = vec![128u8; (width / 2 * height / 2) as usize];
        let frame = Frame {
            width,
            height,
            y: &y,
            y_stride: width,
            chroma: Chroma::I420 { u: &chroma, v: &chroma, u_stride: width / 2, v_stride: width / 2 },
        };
        let out = gpu.texture(width * 2, height * 2);
        let (mut planes, mut upscale) = (None, None);
        let commands = gpu.convert(&mut planes, &mut upscale, &frame, &out).unwrap();
        gpu.submit(commands);
        let upscale = upscale.expect("a bigger texture upscales");
        let [source, stretched, sharpened] = gpu.snapshot(&upscale).unwrap();
        assert_eq!((source.width(), source.height()), (64, 36));
        assert_eq!((sharpened.width(), sharpened.height()), (128, 72));

        let row = 36;
        assert!(
            steepest(&sharpened, row) > steepest(&stretched, row) + 20,
            "FSR's edge {} vs a stretch's {}",
            steepest(&sharpened, row),
            steepest(&stretched, row)
        );
        // Flat parts stay as they came; RCAS leaves only a faint halo by the edge (about 5 of 255), no ringing
        let (dark, bright) = (green(&source, 5, 18), green(&source, 60, 18));
        assert!(green(&sharpened, 10, row).abs_diff(dark) <= 2 && green(&sharpened, 120, row).abs_diff(bright) <= 2);
        assert!((0..128).all(|x| (dark - 8..=bright + 8).contains(&green(&sharpened, x, row))));

        // Half and half: a stretch on the left, FSR on the right, a white line between
        let split = gpu.texture(128, 72);
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        gpu.fsr.record(&gpu.queue, &mut encoder, &upscale, &split, Mode::Split);
        gpu.submit(encoder.finish());
        let split = gpu.read(&split).unwrap();
        assert_eq!(green(&split, 61, row), green(&stretched, 61, row));
        assert_eq!(green(&split, 63, row), 255);
        assert_eq!(green(&split, 66, row), green(&sharpened, 66, row));
    }
}
