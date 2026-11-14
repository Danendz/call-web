//! Video converted to RGBA by the GPU when the window draws with wgpu. Each frame's Y, U and V planes go up as they
//! are, 1.5 bytes a pixel instead of RGBA's 4, and a shader turns them into a texture Slint draws as an image: no
//! per-pixel work on the CPU. Runs on the video threads, on the device and queue Slint draws with; Skia submits on
//! that same queue, so a texture is written only after the frames that drew it before.

use std::sync::OnceLock;
use std::sync::mpsc;

use engine::I420;
use slint::wgpu_30::wgpu;
use slint::{Rgba8Pixel, SharedPixelBuffer};

static CONVERTER: OnceLock<Converter> = OnceLock::new();

/// Called once the window draws with wgpu; until then, and with OpenGL, video is converted on the CPU.
/// `CALL_CPU_VIDEO=1` keeps it there, to compare the two.
pub fn start(device: &wgpu::Device, queue: &wgpu::Queue) {
    if std::env::var_os("CALL_CPU_VIDEO").is_some() {
        return;
    }
    let _ = CONVERTER.get_or_init(|| Converter::new(device.clone(), queue.clone()));
}

pub fn converter() -> Option<&'static Converter> {
    CONVERTER.get()
}

pub struct Converter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// One feed's plane textures, written again by every frame of the same size.
pub struct Planes {
    size: (u32, u32),
    y: wgpu::Texture,
    u: wgpu::Texture,
    v: wgpu::Texture,
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
@fragment fn fragment(in: Out) -> @location(0) vec4f {
    let y = (textureSample(y_plane, planes, in.uv).r - 16.0 / 255.0) * 1.164;
    let u = textureSample(u_plane, planes, in.uv).r - 0.5;
    let v = textureSample(v_plane, planes, in.uv).r - 0.5;
    let rgb = vec3f(y + 1.596 * v, y - 0.391 * u - 0.813 * v, y + 2.018 * u);
    return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);
}
";

impl Converter {
    fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("i420 to rgba"),
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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("i420 planes"),
            entries: &[
                plane(0),
                plane(1),
                plane(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("i420 to rgba"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("i420 to rgba"),
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
                entry_point: Some("fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        // Linear, so the half-size chroma is smoothed rather than blocky; Y is sampled at its texel centers
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("i420 planes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { device, queue, pipeline, layout, sampler }
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
    pub fn convert(&self, planes: &mut Option<Planes>, frame: &I420<'_>, out: &wgpu::Texture) -> bool {
        let (width, height) = (frame.width, frame.height);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let (stride_y, stride_u, stride_v) = frame.strides;
        let fits = |data: &[u8], stride: u32, w: u32, h: u32| {
            stride >= w && (stride as usize * (h as usize - 1) + w as usize) <= data.len()
        };
        if width == 0
            || height == 0
            || out.width() != width
            || out.height() != height
            || !fits(frame.y, stride_y, width, height)
            || !fits(frame.u, stride_u, chroma_width, chroma_height)
            || !fits(frame.v, stride_v, chroma_width, chroma_height)
        {
            return false;
        }
        let planes = match planes {
            Some(planes) if planes.size == (width, height) => planes,
            _ => planes.insert(self.planes(width, height)),
        };
        self.write(&planes.y, frame.y, stride_y, width, height);
        self.write(&planes.u, frame.u, stride_u, chroma_width, chroma_height);
        self.write(&planes.v, frame.v, stride_v, chroma_width, chroma_height);

        let view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("i420 to rgba"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &planes.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        true
    }

    fn planes(&self, width: u32, height: u32) -> Planes {
        let plane = |w, h| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video plane"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let (y, u, v) = (
            plane(width, height),
            plane(width.div_ceil(2), height.div_ceil(2)),
            plane(width.div_ceil(2), height.div_ceil(2)),
        );
        let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let (y_view, u_view, v_view) = (view(&y), view(&u), view(&v));
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("i420 planes"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&y_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&u_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&v_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        Planes { size: (width, height), y, u, v, bind_group }
    }

    fn write(&self, texture: &wgpu::Texture, data: &[u8], stride: u32, width: u32, height: u32) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
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
