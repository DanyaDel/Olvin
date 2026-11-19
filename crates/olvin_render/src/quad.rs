//! Textured-quad batching shared by particles and 2D sprites. Callers push
//! instances in draw order; consecutive ones with the same texture, filter
//! and blend mode share a draw call. The buffers are reused between frames.

use std::collections::HashMap;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::TextureHandle;
use crate::core::GpuMeshes;

const MIN_CAPACITY: usize = 256;
/// Bind-group cache key of the 1×1 white stand-in for "no texture".
const WHITE: u32 = u32::MAX;

/// How a quad's colour is combined with what is behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QuadBlend {
    /// Straight alpha.
    #[default]
    Alpha,
    /// Adds colour × alpha: fire, sparks, glow.
    Additive,
}

/// Texture sampling of a quad batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QuadFilter {
    /// Bilinear on the base level: no mip bleeding across atlas cells.
    #[default]
    Linear,
    /// Bilinear + mipmaps: soft textures seen at different sizes.
    Mipmapped,
    /// Crisp texels: pixel art, tilemaps.
    Nearest,
}

/// One quad: `origin` is the corner at (0, 0), `axis_x` / `axis_y` the edges
/// to (1, 0) and (0, 1). `uv` holds the texture coordinates at (0, 0) in xy
/// and at (1, 1) in zw.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub(crate) struct QuadInstance {
    pub origin: [f32; 4],
    pub axis_x: [f32; 4],
    pub axis_y: [f32; 4],
    pub color: [f32; 4],
    pub uv: [f32; 4],
}

/// What a run of quads has in common.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BatchKey {
    pub texture: Option<TextureHandle>,
    pub filter: QuadFilter,
    pub blend: QuadBlend,
}

struct Batch {
    key: BatchKey,
    range: Range<u32>,
}

/// Uniform of quad.wgsl (mat4 + vec4).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    view_proj: [[f32; 4]; 4],
    flags: [f32; 4],
}

/// Bind group of a (texture, filter) pair, and the texture generation it
/// was built for.
struct CachedBind {
    generation: u64,
    bind: wgpu::BindGroup,
}

/// Pipelines, samplers and buffers of one quad pass; owns the instances
/// pushed since the last [`Self::clear`].
pub(crate) struct QuadBatcher {
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    samplers: [wgpu::Sampler; 3],
    white: wgpu::TextureView,
    uniform: wgpu::Buffer,
    params_bind: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    capacity: usize,
    binds: HashMap<(u32, QuadFilter), CachedBind>,
    instances: Vec<QuadInstance>,
    batches: Vec<Batch>,
    encode_srgb: bool,
}

impl QuadBatcher {
    /// `color` / `samples` / `depth` must match the pass `draw` goes into.
    /// With `depth_test` the quads are hidden by nearer geometry (they never
    /// write depth); without it they paint over everything.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
        depth_test: bool,
    ) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("quad.wgsl"));
        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad params"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<Params>() as u64),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad texture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad"),
            bind_group_layouts: &[Some(&params_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = |blend: QuadBlend| {
            let target = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: match blend {
                        QuadBlend::Alpha => wgpu::BlendFactor::OneMinusSrcAlpha,
                        QuadBlend::Additive => wgpu::BlendFactor::One,
                    },
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("quads"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_quad"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<QuadInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
                            3 => Float32x4, 4 => Float32x4
                        ],
                    })],
                },
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if depth_test {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_quad"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color,
                        blend: Some(target),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = |filter: QuadFilter| {
            let (mag, mipmap, lod_max) = match filter {
                QuadFilter::Linear => (
                    wgpu::FilterMode::Linear,
                    wgpu::MipmapFilterMode::Nearest,
                    0.0,
                ),
                QuadFilter::Mipmapped => (
                    wgpu::FilterMode::Linear,
                    wgpu::MipmapFilterMode::Linear,
                    32.0,
                ),
                QuadFilter::Nearest => (
                    wgpu::FilterMode::Nearest,
                    wgpu::MipmapFilterMode::Nearest,
                    0.0,
                ),
            };
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("quad"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: mag,
                min_filter: mag,
                mipmap_filter: mipmap,
                lod_max_clamp: lod_max,
                ..Default::default()
            })
        };
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("quad white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad params"),
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        Self {
            alpha: pipeline(QuadBlend::Alpha),
            additive: pipeline(QuadBlend::Additive),
            texture_layout,
            samplers: [
                sampler(QuadFilter::Linear),
                sampler(QuadFilter::Mipmapped),
                sampler(QuadFilter::Nearest),
            ],
            white: white.create_view(&Default::default()),
            uniform,
            params_bind,
            buffer: instance_buffer(device, MIN_CAPACITY),
            capacity: MIN_CAPACITY,
            binds: HashMap::new(),
            instances: Vec::new(),
            batches: Vec::new(),
            encode_srgb: !color.is_srgb(),
        }
    }

    /// For an HDR target: no sRGB encoding, a post chain does it.
    pub fn set_hdr_output(&mut self, on: bool, color: wgpu::TextureFormat) {
        self.encode_srgb = !on && !color.is_srgb();
    }

    pub fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn push(&mut self, key: BatchKey, instance: QuadInstance) {
        let at = self.instances.len() as u32;
        match self.batches.last_mut() {
            Some(last) if last.key == key => last.range.end = at + 1,
            _ => self.batches.push(Batch {
                key,
                range: at..at + 1,
            }),
        }
        self.instances.push(instance);
    }

    /// Number of draw calls `draw` issues.
    pub fn batch_count(&self) -> usize {
        self.batches.len()
    }

    /// Uploads the instances and the matrix, and (re)builds the bind groups
    /// of the textures in use.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuMeshes,
        view_proj: Mat4,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&Params {
                view_proj: view_proj.to_cols_array_2d(),
                flags: [f32::from(u8::from(self.encode_srgb)), 0.0, 0.0, 0.0],
            }),
        );
        if self.instances.len() > self.capacity {
            self.capacity = self.instances.len().next_power_of_two();
            self.buffer = instance_buffer(device, self.capacity);
        }
        if !self.instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        for batch in &self.batches {
            let (id, view, generation) = match batch.key.texture {
                None => (WHITE, &self.white, 0),
                Some(h) => match gpu.texture_view(h) {
                    Some((view, generation)) => (h.0, view, generation),
                    None => continue,
                },
            };
            let key = (id, batch.key.filter);
            if self
                .binds
                .get(&key)
                .is_some_and(|c| c.generation == generation)
            {
                continue;
            }
            let sampler = &self.samplers[batch.key.filter as usize];
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("quad texture"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            self.binds.insert(key, CachedBind { generation, bind });
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.batches.is_empty() {
            return;
        }
        pass.set_bind_group(0, &self.params_bind, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        let mut blend = None;
        for batch in &self.batches {
            let id = batch.key.texture.map_or(WHITE, |h| h.0);
            let Some(cached) = self.binds.get(&(id, batch.key.filter)) else {
                continue; // the texture isn't uploaded yet
            };
            if blend != Some(batch.key.blend) {
                blend = Some(batch.key.blend);
                pass.set_pipeline(match batch.key.blend {
                    QuadBlend::Alpha => &self.alpha,
                    QuadBlend::Additive => &self.additive,
                });
            }
            pass.set_bind_group(1, &cached.bind, &[]);
            pass.draw(0..6, batch.range.clone());
        }
    }
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("quad instances"),
        size: (capacity * size_of::<QuadInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Offscreen pixel tests of the quad renderers (particles and sprites) on
/// whatever adapter is there (llvmpipe in CI).
#[cfg(test)]
mod tests {
    use glam::{Vec2, Vec3};
    use olvin_ecs::World;
    use olvin_scene::{Camera, Transform};

    use super::*;
    use crate::{
        Burst, Camera2d, Color, EmitShape, EmitterParams, Materials, ParticleEmitter,
        ParticleRenderer, RenderCore, Shaders, Space2d, Sprite, SpriteAtlas, SpriteRenderer,
        TextureData, Textures, TileSet, Tilemap, camera_view_proj,
    };

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
    const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;

    /// RGB at (x, y) of a rendered frame.
    type Pixels = Box<dyn Fn(u32, u32) -> [u8; 3]>;

    struct Harness {
        core: RenderCore,
        gpu: GpuMeshes,
        textures: Textures,
    }

    impl Harness {
        fn new() -> Option<Self> {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
            let (device, queue) = pollster::block_on(
                adapter.request_device(&wgpu::DeviceDescriptor {
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    ..Default::default()
                }),
            )
            .ok()?;
            Some(Self {
                core: RenderCore::new(device, queue, FORMAT, 1, DEPTH),
                gpu: GpuMeshes::default(),
                textures: Textures::default(),
            })
        }

        /// A texture from rows of RGBA pixels, uploaded.
        fn texture(&mut self, width: u32, height: u32, pixels: &[[u8; 4]]) -> TextureHandle {
            let data = TextureData::rgba8(width, height, pixels.concat(), true);
            let h = self.textures.add(data);
            self.core.upload_assets(
                &mut self.gpu,
                &mut self.textures,
                &mut Materials::default(),
                &mut Shaders::default(),
            );
            h
        }

        /// Draws into a cleared (black) target and returns RGB at (x, y).
        fn render(&self, draw: impl FnOnce(&mut wgpu::RenderPass<'_>)) -> Pixels {
            let device = &self.core.device;
            let rt = wgpu::TextureUsages::RENDER_ATTACHMENT;
            let extent = wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            };
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: rt | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: DEPTH,
                    usage: rt,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let view = color.create_view(&Default::default());
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Discard,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                draw(&mut pass);
            }
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: None,
                    },
                },
                extent,
            );
            self.core.queue.submit([encoder.finish()]);
            readback
                .slice(..)
                .map_async(wgpu::MapMode::Read, |r| r.unwrap());
            device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
            Box::new(move |x, y| {
                let i = ((y * SIZE + x) * 4) as usize;
                [pixels[i], pixels[i + 1], pixels[i + 2]]
            })
        }

        fn sprites(&self, world: &World) -> (Pixels, usize) {
            let mut r = SpriteRenderer::new(&self.core.device, &self.core.queue, FORMAT, 1, DEPTH);
            let n = r.prepare(
                &self.core.device,
                &self.core.queue,
                &self.gpu,
                world,
                (SIZE, SIZE),
            );
            (self.render(|pass| r.draw(pass)), n)
        }
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    fn rgb(c: [u8; 4]) -> [u8; 3] {
        [c[0], c[1], c[2]]
    }

    /// A camera showing 8 × 8 world units on the 64 × 64 target: 8 px a unit.
    fn camera_world() -> World {
        let mut world = World::new();
        world.spawn((Transform::IDENTITY, Camera2d::new(8.0)));
        world
    }

    #[test]
    fn sprites_sort_by_layer_and_y_is_up() {
        let Some(h) = Harness::new() else {
            return;
        };
        let mut world = camera_world();
        // Spawned top layer first: the sort, not the order, decides.
        world.spawn((
            Transform::from_xyz(1.0, 0.0, 0.0),
            Sprite::solid(Color([0.0, 0.0, 1.0, 1.0]), Vec2::splat(4.0)).with_layer(1),
        ));
        world.spawn((
            Transform::IDENTITY,
            Sprite::solid(Color([1.0, 0.0, 0.0, 1.0]), Vec2::splat(4.0)),
        ));
        // Above the origin: appears in the upper half of the image.
        world.spawn((
            Transform::from_xyz(0.0, 3.0, 0.0),
            Sprite::solid(Color([0.0, 1.0, 0.0, 1.0]), Vec2::splat(1.0)),
        ));
        let (px, n) = h.sprites(&world);
        assert_eq!(n, 3);
        assert_eq!(px(20, 32), [255, 0, 0]); // red only (x in -2..-1)
        assert_eq!(px(40, 32), [0, 0, 255]); // blue, over red where they overlap
        assert_eq!(px(34, 32), [0, 0, 255]);
        assert_eq!(px(32, 8), [0, 255, 0]); // y = 3 is 24 px above the centre
        assert_eq!(px(32, 56), [0, 0, 0]);
    }

    #[test]
    fn screen_space_is_pixels_from_the_top_left() {
        let Some(h) = Harness::new() else {
            return;
        };
        let mut world = camera_world();
        let mut ui = Sprite::solid(Color([0.0, 1.0, 0.0, 1.0]), Vec2::new(10.0, 6.0))
            .with_space(Space2d::Screen);
        ui.pivot = Vec2::ZERO;
        world.spawn((Transform::from_xyz(4.0, 2.0, 0.0), ui));
        let (px, _) = h.sprites(&world);
        assert_eq!(px(5, 3), [0, 255, 0]);
        assert_eq!(px(13, 7), [0, 255, 0]);
        assert_eq!(px(15, 3), [0, 0, 0]);
        assert_eq!(px(5, 10), [0, 0, 0]);
        assert_eq!(px(5, 60), [0, 0, 0]);
    }

    #[test]
    fn atlas_regions_flips_and_offscreen_culling() {
        let Some(mut h) = Harness::new() else {
            return;
        };
        // 2 × 2 texels: red green / blue white, rows top to bottom.
        let tex = h.texture(2, 2, &[RED, GREEN, BLUE, [255; 4]]);
        let atlas = SpriteAtlas::grid(tex, [2, 2], 2, 2);
        let mut world = camera_world();
        let mut sprite = atlas.sprite(1, 0.25).unwrap(); // green, 4 × 4 units
        sprite.filter = QuadFilter::Nearest;
        world.spawn((Transform::IDENTITY, sprite.clone()));
        // Far outside the view: culled.
        world.spawn((Transform::from_xyz(100.0, 0.0, 0.0), sprite.clone()));
        let (px, n) = h.sprites(&world);
        assert_eq!(n, 1);
        assert_eq!(px(32, 32), rgb(GREEN));

        let mut flipped = atlas.sprite(0, 0.25).unwrap(); // red region, mirrored
        flipped.filter = QuadFilter::Nearest;
        flipped.flip_x = true;
        let mut world = camera_world();
        world.spawn((Transform::IDENTITY, flipped));
        let (px, _) = h.sprites(&world);
        assert_eq!(px(32, 32), rgb(RED));

        // The whole texture, nearest: quadrants in image orientation.
        let mut whole = Sprite::new(tex, Vec2::splat(4.0));
        whole.filter = QuadFilter::Nearest;
        let mut world = camera_world();
        world.spawn((Transform::IDENTITY, whole));
        let (px, _) = h.sprites(&world);
        assert_eq!(px(24, 24), rgb(RED)); // top-left
        assert_eq!(px(40, 24), rgb(GREEN)); // top-right
        assert_eq!(px(24, 40), rgb(BLUE)); // bottom-left
    }

    #[test]
    fn tilemap_draws_tiles_from_its_top_left() {
        let Some(mut h) = Harness::new() else {
            return;
        };
        let tex = h.texture(2, 1, &[RED, BLUE]);
        let mut map = Tilemap::new(TileSet::new(tex, [2, 1], [1, 1]), 4, 4, Vec2::splat(2.0));
        map.set(0, 0, Some(0), 0); // red, top-left of the map
        map.set(3, 3, Some(1), 0); // blue, bottom-right
        map.set(1, 0, Some(1), Tilemap::FLIP_X); // flipped: still a single colour
        let mut world = camera_world();
        // The map is 8 × 8 units; put its top-left at the view's top-left.
        world.spawn((Transform::from_xyz(-4.0, 4.0, 0.0), map));
        let (px, n) = h.sprites(&world);
        assert_eq!(n, 3);
        assert_eq!(px(4, 4), rgb(RED));
        assert_eq!(px(20, 4), rgb(BLUE));
        assert_eq!(px(60, 60), rgb(BLUE));
        assert_eq!(px(30, 30), [0, 0, 0]);
    }

    #[test]
    fn particles_draw_billboards_and_respect_depth() {
        let Some(h) = Harness::new() else {
            return;
        };
        let mut world = World::new();
        world.spawn((
            Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
            Camera::default(),
        ));
        let mut emitter = ParticleEmitter::new(EmitterParams {
            rate: 0.0,
            bursts: vec![Burst {
                time: 0.0,
                count: 1,
            }],
            shape: EmitShape::Point,
            speed: [0.0, 0.0],
            gravity: Vec3::ZERO,
            size: [2.0, 2.0],
            lifetime: [10.0, 10.0],
            color_over_life: vec![(0.0, [1.0, 0.0, 0.0, 1.0])],
            ..Default::default()
        });
        emitter.update(0.01, &glam::Mat4::IDENTITY);
        assert_eq!(emitter.count(), 1);
        world.spawn((Transform::IDENTITY, emitter));
        let vp = camera_view_proj(&world, 1.0).unwrap();
        let mut r = ParticleRenderer::new(&h.core.device, &h.core.queue, FORMAT, 1, DEPTH);
        assert_eq!(
            r.prepare(&h.core.device, &h.core.queue, &h.gpu, &world, vp),
            1
        );
        let px = h.render(|pass| r.draw(pass));
        // 2 units at distance 5 with a 60 deg fov: about 22 px of 64.
        assert_eq!(px(32, 32), [255, 0, 0]);
        assert_eq!(px(24, 40), [255, 0, 0]);
        assert_eq!(px(4, 4), [0, 0, 0]);
        assert_eq!(r.draw_calls(), 1);
    }
}
