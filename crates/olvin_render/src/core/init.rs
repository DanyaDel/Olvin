//! Device-side setup: [`RenderCore::new`] (pipelines, samplers, default
//! textures), feature-level and output switches, and per-view state.

use std::time::Instant;

use crate::cluster::ClusterTextures;
use crate::pipelines::{
    create_layouts, create_mesh_pipeline, create_shadow_pipeline, create_sky_pipeline,
    shadow_texture,
};
use crate::shader::{DEFAULT_SURFACE_SHADER, for_device, mesh_source};
use crate::{Material, RenderTier, TextureData};

use super::assets::{material_bind_group, material_uniform, upload_texture};
use super::instancing::{BlendSet, InstanceSet};
use super::{DefaultTextures, Globals, RenderCore, ViewData};

impl RenderCore {
    /// `color` / `samples` / `depth` must match the render pass the draws go into.
    /// Non-sRGB colour targets get gamma encoding in the shader.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color: wgpu::TextureFormat,
        samples: u32,
        depth: wgpu::TextureFormat,
    ) -> Self {
        let (layout, material_layout) = create_layouts(&device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let target = (color, samples, depth);
        let tier = tier_of_device(&device);
        let pcss_shader = tier == RenderTier::Full;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mesh"),
            source: wgpu::ShaderSource::Wgsl(
                for_device(&mesh_source(DEFAULT_SURFACE_SHADER), pcss_shader)
                    .into_owned()
                    .into(),
            ),
        });
        let pipeline = create_mesh_pipeline(&device, &pipeline_layout, &module, target, false);
        let blend_pipeline = create_mesh_pipeline(&device, &pipeline_layout, &module, target, true);
        let sky_pipeline = create_sky_pipeline(&device, &layout, target);
        let (shadow_pipeline, shadow_layout) = create_shadow_pipeline(&device);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        // Anisotropy is clamped to 1 where unsupported (WebGL2 without the extension).
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material linear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let point_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material point"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_max_clamp: 0.0,
            ..Default::default()
        });
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let dummy_shadow = shadow_texture(&device, 1);
        let solid = |rgba, srgb| {
            upload_texture(&device, &queue, &TextureData::solid(rgba, srgb), u32::MAX).view
        };
        let defaults = DefaultTextures {
            base: solid([255; 4], true),
            normal: solid([128, 128, 255, 255], false),
            orm: solid([255; 4], false),
            emissive: solid([255; 4], true),
        };
        let fallback = Material::fallback();
        let fallback_material = material_bind_group(
            &device,
            &material_layout,
            &defaults,
            (&linear_sampler, &point_sampler),
            &[],
            &material_uniform(&device, &fallback),
            &fallback,
            false,
        );
        Self {
            fallback_material,
            device,
            queue,
            pipeline,
            blend_pipeline,
            layout,
            material_layout,
            pipeline_layout,
            sky_pipeline,
            shadow_pipeline,
            shadow_layout,
            shadow_sampler,
            linear_sampler,
            point_sampler,
            sky_sampler,
            dummy_shadow,
            defaults,
            target,
            encode_srgb: !color.is_srgb(),
            hdr_output: false,
            tier,
            skin_pipeline: Default::default(),
            pcss_shader,
            time: None,
            start: Instant::now(),
        }
    }

    /// The feature level the core runs at: Compat unless the device is on a
    /// native API (Vulkan, Metal, DX12). [`Self::set_tier`] overrides it.
    pub fn tier(&self) -> RenderTier {
        self.tier
    }

    /// Forces a feature level, e.g. the one the window renderer measured
    /// from its adapter.
    pub fn set_tier(&mut self, tier: RenderTier) {
        self.tier = tier;
    }

    /// For a core drawing into an HDR target ([`crate::HDR_FORMAT`]): the
    /// shaders skip exposure, tonemapping, clamping and sRGB encoding, which
    /// the post-processing chain does instead.
    pub fn set_hdr_output(&mut self, on: bool) {
        self.hdr_output = on;
    }

    /// Seconds for surface shaders' `in.time`. Until it's called, the time
    /// since this core was created.
    pub fn set_time(&mut self, seconds: f32) {
        self.time = Some(seconds);
    }

    pub fn create_view(&self) -> ViewData {
        let uniform = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let globals = uniform("olvin globals", size_of::<Globals>() as u64);
        let clusters = ClusterTextures::new(&self.device);
        let bind = self.main_bind_group(
            &globals,
            &self.dummy_shadow,
            &self.dummy_shadow,
            &self.defaults.base,
            &clusters,
        );
        ViewData {
            globals,
            bind,
            main: InstanceSet::new(&self.device),
            blend: BlendSet::new(&self.device),
            staging: Vec::new(),
            cascades: Vec::new(),
            cascade_count: 0,
            shadow_tile: 1,
            shadow_map: None,
            local_map: None,
            local_tiles: Vec::new(),
            local_count: 0,
            local_tile: 1,
            sky_bound: None,
            sky_active: false,
            retro_nearest: false,
            retro_global: false,
            clusters,
        }
    }

    pub(super) fn main_bind_group(
        &self,
        globals: &wgpu::Buffer,
        shadow: &wgpu::TextureView,
        local: &wgpu::TextureView,
        sky: &wgpu::TextureView,
        clusters: &ClusterTextures,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("olvin globals"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(shadow),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(sky),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sky_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(local),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&clusters.lights_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&clusters.grid_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(&clusters.index_view),
                },
            ],
        })
    }
}

/// Native APIs run Full; GL / WebGL stay Compat (see [`crate::tier_of`]).
fn tier_of_device(device: &wgpu::Device) -> RenderTier {
    if device.adapter_info().backend == wgpu::Backend::Gl {
        RenderTier::Compat
    } else {
        RenderTier::Full
    }
}
