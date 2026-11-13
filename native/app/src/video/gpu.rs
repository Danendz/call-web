//! Video converted on the GPU, when the window draws through wgpu on Metal or D3D12. The video thread uploads a
//! frame's YUV planes and one render pass turns them into an RGBA texture that Slint draws as is: nothing is
//! converted on the CPU, and the planes are 2.7× smaller than RGBA. Skia draws on the same queue, after the pass was
//! submitted, so a texture is always complete when it's shown.
//!
//! Not on Vulkan: Skia submits to the raw queue from the UI thread, and Vulkan needs every submit to one queue
//! synchronized, which the video thread's wouldn't be. Without a device, frames take the CPU path.

use std::sync::{Arc, Mutex, OnceLock};

use engine::Planes;
use slint::wgpu_30::wgpu;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// The window's device, once its renderer is set up
static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Built on the first frame, so starting the app doesn't wait for the shader
    pipelines: OnceLock<Pipelines>,
}

struct Pipelines {
    i420: wgpu::RenderPipeline,
    nv12: wgpu::RenderPipeline,
}

/// The window's device when its renderer is set up, or `None` once it's torn down.
pub fn set_device(device: Option<(wgpu::Device, wgpu::Queue)>) {
    let gpu = device.map(|(device, queue)| Arc::new(Gpu { device, queue, pipelines: OnceLock::new() }));
    if let Ok(mut current) = GPU.lock() {
        *current = gpu;
    }
}

pub fn get() -> Option<Arc<Gpu>> {
    GPU.lock().ok()?.clone()
}

/// The frames a target can take: another size or format needs another target.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    width: u32,
    height: u32,
    nv12: bool,
}

impl Layout {
    pub fn of(planes: &Planes<'_>) -> Self {
        match *planes {
            Planes::I420 { width, height, .. } => Self { width, height, nv12: false },
            Planes::Nv12 { width, height, .. } => Self { width, height, nv12: true },
        }
    }
}

/// One converted frame: the planes it was made from and the RGBA texture they were drawn into. Kept and written
/// again, like the CPU path's buffers.
#[derive(Clone)]
pub struct Target {
    layout: Layout,
    device: wgpu::Device,
    planes: Vec<wgpu::Texture>,
    rgba: wgpu::Texture,
    bind: wgpu::BindGroup,
}

impl Target {
    pub fn image(&self) -> Image {
        Image::try_from(self.rgba.clone()).unwrap_or_default()
    }
}

impl Gpu {
    /// Whether `target` can take a frame of `layout`: same layout, and made on this device.
    pub fn fits(&self, target: &Target, layout: Layout) -> bool {
        target.layout == layout && target.device == self.device
    }

    pub fn target(&self, layout: Layout) -> Target {
        let (width, height) = (layout.width, layout.height);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let texture = |width, height, format, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("video"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let plane = |width, height, format| {
            texture(width, height, format, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST)
        };
        let planes = if layout.nv12 {
            vec![
                plane(width, height, wgpu::TextureFormat::R8Unorm),
                plane(chroma_width, chroma_height, wgpu::TextureFormat::Rg8Unorm),
            ]
        } else {
            vec![
                plane(width, height, wgpu::TextureFormat::R8Unorm),
                plane(chroma_width, chroma_height, wgpu::TextureFormat::R8Unorm),
                plane(chroma_width, chroma_height, wgpu::TextureFormat::R8Unorm),
            ]
        };
        // Slint draws it, and screenshots copy it out
        let rgba = texture(
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
        );
        let views: Vec<_> = planes.iter().map(|p| p.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        let entries: Vec<_> = (0..)
            .zip(&views)
            .map(|(binding, view)| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) })
            .collect();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video"),
            layout: &self.pipeline(layout.nv12).get_bind_group_layout(0),
            entries: &entries,
        });
        Target { layout, device: self.device.clone(), planes, rgba, bind }
    }

    /// Uploads the planes into `target` and draws them as RGBA.
    pub fn convert(&self, target: &Target, planes: &Planes<'_>) {
        match *planes {
            Planes::I420 { y, u, v, strides, .. } => {
                for ((texture, data), stride) in target.planes.iter().zip([y, u, v]).zip(strides) {
                    self.upload(texture, data, stride);
                }
            }
            Planes::Nv12 { y, uv, strides, .. } => {
                for ((texture, data), stride) in target.planes.iter().zip([y, uv]).zip(strides) {
                    self.upload(texture, data, stride);
                }
            }
        }
        let view = target.rgba.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("video") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("video"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    // Every pixel is drawn over
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
            pass.set_pipeline(self.pipeline(target.layout.nv12));
            pass.set_bind_group(0, &target.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    fn upload(&self, texture: &wgpu::Texture, data: &[u8], stride: u32) {
        let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None };
        self.queue.write_texture(texture.as_image_copy(), data, layout, texture.size());
    }

    fn pipeline(&self, nv12: bool) -> &wgpu::RenderPipeline {
        let pipelines = self.pipelines.get_or_init(|| {
            let module = self.device.create_shader_module(wgpu::include_wgsl!("yuv.wgsl"));
            let pipeline = |fragment| {
                self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("video"),
                    // Each pipeline gets the bindings its fragment shader reads
                    layout: None,
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs"),
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
            Pipelines { i420: pipeline("fs_i420"), nv12: pipeline("fs_nv12") }
        });
        if nv12 { &pipelines.nv12 } else { &pipelines.i420 }
    }
}

/// The pixels of a frame the GPU converted, copied back for a screenshot. Waits for the GPU, a few milliseconds.
pub fn read(image: &Image) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let gpu = get()?;
    let texture = image.to_wgpu_30_texture()?;
    let (width, height) = (texture.width(), texture.height());
    let packed = width as usize * 4;
    let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("screenshot") });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
        },
        texture.size(),
    );
    gpu.queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (mapped, done) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = mapped.send(result);
    });
    gpu.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok()?;
    done.recv().ok()?.ok()?;
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    let data = slice.get_mapped_range().ok()?;
    for (to, from) in pixels.make_mut_bytes().chunks_exact_mut(packed).zip(data.chunks(row as usize)) {
        to.copy_from_slice(from.get(..packed)?);
    }
    Some(pixels)
}
