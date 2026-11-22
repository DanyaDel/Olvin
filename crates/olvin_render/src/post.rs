//! Post-processing on the HDR frame (Full tier): the scene is drawn into an
//! `Rgba16Float` target (plus a depth texture the chain can read) without
//! tonemapping, then a short chain of fullscreen passes computes ambient
//! occlusion from the depth, adds bloom, applies exposure and the tonemapper,
//! grades the colour (LUT, lift / gamma / gain, saturation, contrast), and
//! writes the surface (through FXAA when it is on). Compat hardware skips
//! all of this and tonemaps in the mesh shader.

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::{GpuMeshes, RenderSettings, TextureData, TextureHandle, Tonemapping};

/// What the scene is drawn into before post-processing.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The depth texture the scene is drawn with ([`PostProcess::depth_view`]).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
/// Ambient occlusion is stored in one 8-bit channel.
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Ground-truth ambient occlusion (GTAO) from the depth buffer: darkens
/// creases, contact points and the undersides of objects. Multiplied onto
/// the lit colour (not only the ambient term, which is not available to a
/// post pass), so it fades out on very bright pixels. Costs two fullscreen
/// passes: 20 depth reads per pixel, then a depth-aware 4×4 blur.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmbientOcclusion {
    /// Reach of the occlusion in world units (metres).
    pub radius: f32,
    /// Exponent applied to the visibility: 1 is plain GTAO, higher is darker.
    pub strength: f32,
    /// Slack against self-occlusion on flat, slanted surfaces (cosine of the
    /// horizon angle that is ignored).
    pub bias: f32,
}

impl Default for AmbientOcclusion {
    fn default() -> Self {
        Self {
            radius: 1.0,
            strength: 1.2,
            bias: 0.05,
        }
    }
}

/// Colour grading after tonemapping, in display (sRGB-encoded) colour:
/// lift / gamma / gain, then contrast and saturation, then the LUT. The
/// default changes nothing and costs nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGrade {
    /// A LUT strip (see [`lut_strip`]), uploaded as
    /// [`TextureFormat::Rgba8Linear`](crate::TextureFormat): `size` slices of
    /// `size`² texels side by side, blue picks the slice, red runs along a
    /// slice, green down it.
    pub lut: Option<TextureHandle>,
    /// 0 = the graded colour as it is, 1 = fully through the LUT.
    pub lut_strength: f32,
    /// 1 = unchanged, 0 = grey.
    pub saturation: f32,
    /// Around mid grey; 1 = unchanged.
    pub contrast: f32,
    /// Added to the shadows (0 = unchanged).
    pub lift: [f32; 3],
    /// Midtone curve per channel (1 = unchanged, > 1 brightens).
    pub gamma: [f32; 3],
    /// Multiplies the highlights (1 = unchanged).
    pub gain: [f32; 3],
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            lut: None,
            lut_strength: 1.0,
            saturation: 1.0,
            contrast: 1.0,
            lift: [0.0; 3],
            gamma: [1.0; 3],
            gain: [1.0; 3],
        }
    }
}

impl ColorGrade {
    /// Nothing to apply (the grading is skipped altogether).
    pub fn is_neutral(&self) -> bool {
        let has_lut = self.lut.is_some() && self.lut_strength > 0.0;
        !has_lut
            && self.saturation == 1.0
            && self.contrast == 1.0
            && self.lift == [0.0; 3]
            && self.gamma == [1.0; 3]
            && self.gain == [1.0; 3]
    }
}

/// A LUT strip of `size`³ colours (4 to 64): `f` maps each sRGB-encoded input
/// colour to its graded colour (both 0..1). Upload it with
/// `Textures::add`; blue picks the slice, red runs along it, green down it.
pub fn lut_strip(size: u32, f: impl Fn([f32; 3]) -> [f32; 3]) -> TextureData {
    let n = size.clamp(4, 64);
    let max = (n - 1) as f32;
    let mut pixels = Vec::with_capacity((n * n * n * 4) as usize);
    for g in 0..n {
        for b in 0..n {
            for r in 0..n {
                let out = f([r as f32 / max, g as f32 / max, b as f32 / max]);
                pixels.extend(out.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
                pixels.push(255);
            }
        }
    }
    TextureData::rgba8(n * n, n, pixels, false)
}

/// The LUT strip that changes nothing; the starting point for grading in an
/// image editor, and what a fresh LUT slot holds.
pub fn identity_lut(size: u32) -> TextureData {
    lut_strip(size, |c| c)
}

/// Post-processing switches of [`RenderSettings`]. Like bloom, they need the
/// chain of [`RenderTier::Full`](crate::RenderTier::Full) in the window
/// renderer and are ignored elsewhere.
#[derive(Clone, Debug, PartialEq)]
pub struct PostSettings {
    /// FXAA 3.11 on the tonemapped image: smooths aliased edges for the cost
    /// of one extra fullscreen pass.
    pub fxaa: bool,
    /// Ambient occlusion; off by default: it is the one effect with a real
    /// cost and it changes the look of every scene.
    pub ao: Option<AmbientOcclusion>,
    /// Colour grading (LUT and simple controls); neutral by default.
    pub grade: ColorGrade,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            fxaa: true,
            ao: None,
            grade: ColorGrade::default(),
        }
    }
}

/// Uniform of post.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    /// threshold, intensity, exposure, ACES.
    bloom_exposure: [f32; 4],
    /// sRGB encode, -, -, -.
    output: [f32; 4],
    /// Blur step in uv.
    blur: [f32; 4],
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    /// AO radius, strength, bias, 1: AO on.
    ao: [f32; 4],
    /// LUT strength, saturation, contrast, 1: grading on.
    grade: [f32; 4],
    /// Lift rgb, -; gamma rgb, 1: a LUT is bound; gain rgb, -.
    lift: [f32; 4],
    gamma: [f32; 4],
    gain: [f32; 4],
}

/// Textures and bind groups that depend on the frame size.
struct Targets {
    size: (u32, u32),
    hdr: wgpu::TextureView,
    depth: wgpu::TextureView,
    half: wgpu::TextureView,
    quarter: wgpu::TextureView,
    quarter_blur: wgpu::TextureView,
    // One bind group per pass.
    bright: wgpu::BindGroup,
    down: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    composite: wgpu::BindGroup,
    ao_raw: wgpu::TextureView,
    ao: wgpu::BindGroup,
    ao_blur: wgpu::BindGroup,
    /// Group 1 (depth, AO, LUT) for the passes before the composite (the
    /// blurred AO is still being written there), and from the composite on.
    aux_pre: wgpu::BindGroup,
    aux_post: wgpu::BindGroup,
    ao_blurred: wgpu::TextureView,
    /// Tonemapped frame in the output's format, input of FXAA.
    ldr: wgpu::TextureView,
    fxaa: wgpu::BindGroup,
}

/// The HDR target and the bloom / tonemap passes. Owned by the window
/// renderer, which draws the scene into [`Self::hdr_view`].
pub struct PostProcess {
    layout: wgpu::BindGroupLayout,
    aux_layout: wgpu::BindGroupLayout,
    /// Stand-ins for AO (while it is being made) and the LUT (none).
    empty_ao: wgpu::TextureView,
    empty_lut: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bright: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
    ao: wgpu::RenderPipeline,
    ao_blur: wgpu::RenderPipeline,
    /// Parameter buffers: main (composite, bright-pass), horizontal and
    /// vertical blur.
    main: wgpu::Buffer,
    horizontal: wgpu::Buffer,
    vertical: wgpu::Buffer,
    lut: Option<wgpu::TextureView>,
    /// Upload generation of `lut`.
    lut_generation: u64,
    encode_srgb: bool,
    output: wgpu::TextureFormat,
    targets: Option<Targets>,
}

impl PostProcess {
    /// `output` is the format of the surface the chain writes to.
    pub fn new(device: &wgpu::Device, output: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("post.wgsl"));
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty,
            count: None,
        };
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<Params>() as u64),
                    },
                ),
                entry(1, texture),
                entry(2, texture),
                entry(
                    3,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
            ],
        });
        let aux_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post aux"),
            entries: &[
                // The depth is read with `textureLoad` as a plain float texture
                // (`texture_depth_2d` loads don't translate to GLSL).
                entry(
                    0,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(1, texture),
                entry(2, texture),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&layout), Some(&aux_layout)],
            immediate_size: 0,
        });
        let pipeline = |fragment: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fragment),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_post"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let uniform = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let one = |format| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("post empty"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            t.create_view(&Default::default())
        };
        Self {
            ao: pipeline("fs_ao", AO_FORMAT),
            ao_blur: pipeline("fs_ao_blur", AO_FORMAT),
            empty_ao: one(AO_FORMAT),
            empty_lut: one(wgpu::TextureFormat::Rgba8Unorm),
            aux_layout,
            bright: pipeline("fs_bright", HDR_FORMAT),
            down: pipeline("fs_down", HDR_FORMAT),
            blur: pipeline("fs_blur", HDR_FORMAT),
            composite: pipeline("fs_composite", output),
            fxaa: pipeline("fs_fxaa", output),
            main: uniform("post params"),
            horizontal: uniform("post blur h"),
            vertical: uniform("post blur v"),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("post"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            layout,
            lut: None,
            lut_generation: 0,
            encode_srgb: !output.is_srgb(),
            output,
            targets: None,
        }
    }

    /// (Re)creates the frame-sized targets when the size changed. Call
    /// before [`Self::hdr_view`] each frame.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let target = |label, format, (w, h): (u32, u32)| {
            let (w, h) = (w.max(1), h.max(1));
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let hdr = target("post hdr", HDR_FORMAT, size);
        let half = target("post half", HDR_FORMAT, (size.0 / 2, size.1 / 2));
        let quarter = target("post quarter", HDR_FORMAT, (size.0 / 4, size.1 / 4));
        let quarter_blur = target("post quarter blur", HDR_FORMAT, (size.0 / 4, size.1 / 4));
        let ldr = target("post ldr", self.output, size);
        let ao_raw = target("post ao", AO_FORMAT, size);
        let ao_blurred = target("post ao blurred", AO_FORMAT, size);
        let depth = target("post depth", DEPTH_FORMAT, size);
        let bind =
            |params: &wgpu::Buffer, source: &wgpu::TextureView, bloom: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("post"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(bloom),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            };
        let (aux_pre, aux_post) = self.aux_groups(device, &depth, &ao_blurred);
        self.targets = Some(Targets {
            size,
            // The unused second texture is one the pass doesn't write to.
            bright: bind(&self.main, &hdr, &quarter_blur),
            down: bind(&self.main, &half, &quarter_blur),
            blur_h: bind(&self.horizontal, &quarter, &half),
            blur_v: bind(&self.vertical, &quarter_blur, &half),
            composite: bind(&self.main, &hdr, &quarter),
            fxaa: bind(&self.main, &ldr, &quarter),
            ao: bind(&self.main, &hdr, &quarter),
            ao_blur: bind(&self.main, &ao_raw, &quarter),
            ao_raw,
            aux_pre,
            aux_post,
            ao_blurred,
            ldr,
            half,
            quarter,
            quarter_blur,
            hdr,
            depth,
        });
    }

    /// Group 1 of the passes: the depth, the AO (`empty_ao` while it is
    /// made) and the LUT.
    fn aux_groups(
        &self,
        device: &wgpu::Device,
        depth: &wgpu::TextureView,
        ao: &wgpu::TextureView,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let lut = self.lut.as_ref().unwrap_or(&self.empty_lut);
        Self::aux_groups_for(device, &self.aux_layout, &self.empty_ao, lut, depth, ao)
    }

    fn aux_groups_for(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        empty_ao: &wgpu::TextureView,
        lut: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        ao: &wgpu::TextureView,
    ) -> (wgpu::BindGroup, wgpu::BindGroup) {
        let group = |ao: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post aux"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(ao),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(lut),
                    },
                ],
            })
        };
        (group(empty_ao), group(ao))
    }

    /// Binds the grading LUT of `settings` (call each frame before
    /// [`Self::run`], with the registries' GPU copies): cheap unless the
    /// texture changed.
    pub fn sync_lut(&mut self, device: &wgpu::Device, gpu: &GpuMeshes, settings: &RenderSettings) {
        let found = settings
            .post
            .grade
            .lut
            .and_then(|h| gpu.texture_view(h))
            .filter(|(view, _)| view.texture().height() > 1);
        let generation = found.map_or(0, |(_, g)| g);
        if generation == self.lut_generation && found.is_some() == self.lut.is_some() {
            return;
        }
        self.lut = found.map(|(view, _)| view.clone());
        self.lut_generation = generation;
        if let Some(t) = &mut self.targets {
            let (pre, post) = Self::aux_groups_for(
                device,
                &self.aux_layout,
                &self.empty_ao,
                self.lut.as_ref().unwrap_or(&self.empty_lut),
                &t.depth,
                &t.ao_blurred,
            );
            t.aux_pre = pre;
            t.aux_post = post;
        }
    }

    /// The depth texture to draw the scene with (after [`Self::resize`]); the
    /// scene pass must store it when `settings.post.ao` is on, see
    /// [`begin_scene_pass`](crate::renderer::begin_scene_pass).
    pub fn depth_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.depth)
    }

    /// The target to draw the scene into (after [`Self::resize`]).
    pub fn hdr_view(&self) -> Option<&wgpu::TextureView> {
        self.targets.as_ref().map(|t| &t.hdr)
    }

    /// Records the chain from the HDR frame into `output`; `view_proj` is the
    /// camera the scene was drawn with. AO runs when `settings.post.ao` is
    /// set (the scene pass kept the depth), bloom when `settings.bloom` is,
    /// FXAA when `settings.post.fxaa` is; exposure and the tonemapper always.
    pub fn run(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        output: &wgpu::TextureView,
        settings: &RenderSettings,
        view_proj: Mat4,
    ) {
        let Some(t) = &self.targets else {
            return;
        };
        let bloom = settings.bloom.filter(|b| b.intensity > 0.0);
        let ao = settings
            .post
            .ao
            .filter(|a| a.radius > 0.0 && a.strength > 0.0);
        let grade = &settings.post.grade;
        let has_lut = self.lut.is_some() && grade.lut_strength > 0.0;
        let (w, h) = (t.size.0 as f32, t.size.1 as f32);
        let main = Params {
            bloom_exposure: [
                bloom.map_or(1.0, |b| b.threshold.max(0.0)),
                bloom.map_or(0.0, |b| b.intensity),
                settings.exposure.max(0.0),
                if settings.tonemapping == Tonemapping::Aces {
                    1.0
                } else {
                    0.0
                },
            ],
            output: [if self.encode_srgb { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            blur: [0.0; 4],
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            ao: ao.map_or([0.0; 4], |a| {
                [a.radius, a.strength, a.bias.clamp(0.0, 0.5), 1.0]
            }),
            grade: [
                if has_lut {
                    grade.lut_strength.min(1.0)
                } else {
                    0.0
                },
                grade.saturation.max(0.0),
                grade.contrast.max(0.0),
                if grade.is_neutral() { 0.0 } else { 1.0 },
            ],
            lift: [grade.lift[0], grade.lift[1], grade.lift[2], 0.0],
            gamma: [
                grade.gamma[0].max(0.01),
                grade.gamma[1].max(0.01),
                grade.gamma[2].max(0.01),
                if has_lut { 1.0 } else { 0.0 },
            ],
            gain: [grade.gain[0], grade.gain[1], grade.gain[2], 0.0],
        };
        queue.write_buffer(&self.main, 0, bytemuck::bytes_of(&main));
        if ao.is_some() {
            pass(encoder, &t.ao_raw, &self.ao, &t.ao, &t.aux_pre);
            pass(
                encoder,
                &t.ao_blurred,
                &self.ao_blur,
                &t.ao_blur,
                &t.aux_pre,
            );
        }
        if bloom.is_some() {
            let blur = |x: f32, y: f32| Params {
                blur: [x, y, 0.0, 0.0],
                ..main
            };
            let (qw, qh) = ((w / 4.0).floor().max(1.0), (h / 4.0).floor().max(1.0));
            queue.write_buffer(
                &self.horizontal,
                0,
                bytemuck::bytes_of(&blur(1.0 / qw, 0.0)),
            );
            queue.write_buffer(&self.vertical, 0, bytemuck::bytes_of(&blur(0.0, 1.0 / qh)));
            pass(encoder, &t.half, &self.bright, &t.bright, &t.aux_pre);
            pass(encoder, &t.quarter, &self.down, &t.down, &t.aux_pre);
            pass(encoder, &t.quarter_blur, &self.blur, &t.blur_h, &t.aux_pre);
            pass(encoder, &t.quarter, &self.blur, &t.blur_v, &t.aux_pre);
        }
        if settings.post.fxaa {
            pass(encoder, &t.ldr, &self.composite, &t.composite, &t.aux_post);
            pass(encoder, output, &self.fxaa, &t.fxaa, &t.aux_post);
        } else {
            pass(encoder, output, &self.composite, &t.composite, &t.aux_post);
        }
    }
}

/// One fullscreen triangle into `target`.
fn pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    aux: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("olvin post"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        ..Default::default()
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bind, &[]);
    pass.set_bind_group(1, aux, &[]);
    pass.draw(0..3, 0..1);
}
