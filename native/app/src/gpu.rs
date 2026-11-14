//! Video converted to RGBA by the GPU when the window draws with wgpu. Each frame's Y, U and V planes go up as they
//! are, 1.5 bytes a pixel instead of RGBA's 4, and a shader turns them into a texture Slint draws as an image: no
//! per-pixel work on the CPU. Runs on the video threads, on the device and queue Slint draws with; Skia submits on
//! that same queue, so a texture is written only after the frames that drew it before.
//!
//! Only on Metal and D3D12, whose queues take submits from any thread. Skia submits to Vulkan's raw queue from the UI
//! thread, and Vulkan needs every submit to one queue synchronized, which the video threads' wouldn't be.

use std::sync::{Arc, Mutex, mpsc};

use engine::{Chroma, Planes as Frame};
use slint::wgpu_30::wgpu;
use slint::{Rgba8Pixel, SharedPixelBuffer};

static CONVERTER: Mutex<Option<Arc<Converter>>> = Mutex::new(None);

/// Called once the window draws with wgpu; until then, on Vulkan and with OpenGL, video is converted on the CPU.
/// `CALL_CPU_VIDEO=1` keeps it there, to compare the two.
pub fn start(device: &wgpu::Device, queue: &wgpu::Queue) {
    let backend = device.adapter_info().backend;
    let converter = (matches!(backend, wgpu::Backend::Metal | wgpu::Backend::Dx12)
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
    device: wgpu::Device,
    queue: wgpu::Queue,
    i420: Pipeline,
    nv12: Pipeline,
    sampler: wgpu::Sampler,
}

struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

/// One feed's plane textures, written again by every frame of the same size and format on the same device.
pub struct Planes {
    device: wgpu::Device,
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
        Self { device, queue, i420, nv12, sampler }
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
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

    /// Converts `frame` into `out`, a [`Self::texture`] of the frame's size. A frame whose planes are shorter than
    /// its size says is skipped (false): wgpu treats a short write as a bug and panics.
    pub fn convert(&self, planes: &mut Option<Planes>, frame: &Frame<'_>, out: &wgpu::Texture) -> bool {
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
        if width == 0
            || height == 0
            || out.width() != width
            || out.height() != height
            || !fits(frame.y, frame.y_stride, width, height)
            || !chroma_fits
        {
            return false;
        }
        let nv12 = matches!(frame.chroma, Chroma::Nv12 { .. });
        let planes = match planes {
            Some(planes) if planes.size == (width, height) && planes.nv12 == nv12 && planes.device == self.device => {
                planes
            }
            _ => planes.insert(self.planes(width, height, nv12)),
        };
        let data = match frame.chroma {
            Chroma::I420 { u, v, u_stride, v_stride } => vec![(frame.y, frame.y_stride), (u, u_stride), (v, v_stride)],
            Chroma::Nv12 { uv, stride } => vec![(frame.y, frame.y_stride), (uv, stride)],
        };
        for (texture, (data, stride)) in planes.textures.iter().zip(data) {
            self.write(texture, data, stride);
        }

        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
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
        self.queue.submit(Some(encoder.finish()));
        true
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
        Planes { device: self.device.clone(), size: (width, height), nv12, textures, bind_group }
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
        let submitted = self.queue.submit(Some(encoder.finish()));
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
