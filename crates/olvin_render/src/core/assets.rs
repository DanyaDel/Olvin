//! GPU assets: mesh and texture uploads, material uniforms and bind groups,
//! and compiling surface shaders into pipelines.

use crate::pipelines::create_mesh_pipeline;
use crate::{
    BlendMode, Material, Materials, MeshData, Meshes, Shaders, TextureData, TextureHandle,
    Textures, Vertex,
};

use super::{
    DefaultTextures, GpuMaterial, GpuMesh, GpuMeshes, GpuTexture, RenderCore, SurfacePipeline,
};

impl RenderCore {
    fn material_uniform(&self, material: &Material) -> wgpu::Buffer {
        material_uniform(&self.device, material)
    }

    fn material_bind_group(
        &self,
        textures: &[Option<GpuTexture>],
        uniform: &wgpu::Buffer,
        material: &Material,
    ) -> wgpu::BindGroup {
        let samplers = (&self.linear_sampler, &self.point_sampler);
        material_bind_group(
            &self.device,
            &self.material_layout,
            &self.defaults,
            samplers,
            textures,
            uniform,
            material,
        )
    }

    /// Uploads meshes added to `meshes` since the last call.
    pub fn upload(&self, gpu: &mut GpuMeshes, meshes: &mut Meshes) {
        use wgpu::util::DeviceExt;
        for (handle, data) in meshes.pending.drain(..) {
            let MeshData { vertices, indices } = &data;
            // Nothing to draw (and wgpu cannot slice a zero-size buffer): an
            // emptied mesh leaves its slot empty, which every pass skips.
            if vertices.is_empty() || indices.is_empty() {
                if let Some(slot) = gpu.meshes.get_mut(handle.0 as usize) {
                    *slot = None;
                }
                continue;
            }
            let (center, radius) = data.bounding_sphere();
            let mesh = GpuMesh {
                vertices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh vertices"),
                        contents: bytemuck::cast_slice::<Vertex, u8>(vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                indices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("mesh indices"),
                        contents: bytemuck::cast_slice::<u32, u8>(indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                index_count: indices.len() as u32,
                center,
                radius,
            };
            put(&mut gpu.meshes, handle.0, mesh);
        }
    }

    /// Uploads what changed in the asset registries since the last call:
    /// compiles surface shaders (a failure keeps the last good pipeline and
    /// is reported by `Shaders::error`; nothing panics), uploads textures
    /// with mipmaps, and (re)builds material bind groups, including those of
    /// materials whose textures were replaced. Call it next to [`Self::upload`],
    /// before `prepare`.
    pub fn upload_assets(
        &self,
        gpu: &mut GpuMeshes,
        textures: &mut Textures,
        materials: &mut Materials,
        shaders: &mut Shaders,
    ) {
        let changed = !shaders.pending.is_empty() || !materials.pending.is_empty();
        for (handle, source) in std::mem::take(&mut shaders.pending) {
            match self.compile_surface(&source) {
                Ok(pipeline) => put(&mut gpu.pipelines, handle.0, pipeline),
                Err(e) => {
                    log::warn!("surface shader {}: {e}", handle.0);
                    shaders.set_error(handle, Some(e));
                }
            }
        }

        let max = self.device.limits().max_texture_dimension_2d;
        let mut replaced = Vec::new();
        for (handle, data) in textures.pending.drain(..) {
            gpu.generation += 1;
            let mut texture = upload_texture(&self.device, &self.queue, &data, max);
            texture.generation = gpu.generation;
            if matches!(gpu.textures.get(handle.0 as usize), Some(Some(_))) {
                replaced.push(handle);
            }
            put(&mut gpu.textures, handle.0, texture);
        }
        if !replaced.is_empty() {
            // New texture objects under existing materials: rebind those (an asset reload).
            for slot in 0..gpu.materials.len() {
                let Some(m) = &gpu.materials[slot] else {
                    continue;
                };
                if m.material
                    .textures()
                    .iter()
                    .flatten()
                    .any(|t| replaced.contains(t))
                {
                    let bind = self.material_bind_group(&gpu.textures, &m.uniform, &m.material);
                    if let Some(m) = &mut gpu.materials[slot] {
                        m.bind = bind;
                    }
                }
            }
        }

        for (handle, material) in materials.pending.drain(..) {
            let uniform = match gpu.materials.get(handle.0 as usize) {
                Some(Some(existing)) => {
                    self.queue.write_buffer(
                        &existing.uniform,
                        0,
                        bytemuck::bytes_of(&material.uniform()),
                    );
                    existing.uniform.clone()
                }
                _ => self.material_uniform(&material),
            };
            let bind = self.material_bind_group(&gpu.textures, &uniform, &material);
            put(
                &mut gpu.materials,
                handle.0,
                GpuMaterial {
                    material,
                    uniform,
                    bind,
                },
            );
        }

        if changed {
            self.build_blend_pipelines(gpu);
        }
    }

    /// Builds the alpha-blended pipeline of every shader a blended material
    /// uses and that doesn't have one yet.
    fn build_blend_pipelines(&self, gpu: &mut GpuMeshes) {
        for slot in 0..gpu.materials.len() {
            let Some(m) = &gpu.materials[slot] else {
                continue;
            };
            let Some(shader) = m
                .material
                .shader
                .filter(|_| m.material.blend == BlendMode::Alpha)
            else {
                continue;
            };
            if let Some(Some(p)) = gpu.pipelines.get_mut(shader.0 as usize)
                && p.blend.is_none()
            {
                p.blend = Some(create_mesh_pipeline(
                    &self.device,
                    &self.pipeline_layout,
                    &p.module,
                    self.target,
                    true,
                ));
            }
        }
    }

    /// Builds a mesh pipeline from a full template source, catching wgpu's
    /// errors instead of letting them reach the device's panic handler.
    fn compile_surface(&self, source: &str) -> Result<SurfacePipeline, String> {
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("surface shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let opaque = create_mesh_pipeline(
            &self.device,
            &self.pipeline_layout,
            &module,
            self.target,
            false,
        );
        let validation = pollster::block_on(validation.pop());
        let internal = pollster::block_on(internal.pop());
        match validation.or(internal) {
            None => Ok(SurfacePipeline {
                module,
                opaque,
                blend: None,
            }),
            Some(e) => Err(e.to_string()),
        }
    }
}

pub(super) fn material_uniform(device: &wgpu::Device, material: &Material) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("material"),
        contents: bytemuck::bytes_of(&material.uniform()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Material uniform + its textures (defaults for missing ones) + sampler.
pub(super) fn material_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    defaults: &DefaultTextures,
    (linear, point): (&wgpu::Sampler, &wgpu::Sampler),
    textures: &[Option<GpuTexture>],
    uniform: &wgpu::Buffer,
    material: &Material,
) -> wgpu::BindGroup {
    let view = |handle: Option<TextureHandle>, default| {
        handle
            .and_then(|h| textures.get(h.0 as usize))
            .and_then(Option::as_ref)
            .map_or(default, |t: &GpuTexture| &t.view)
    };
    let entry = |binding, view| wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    };
    let sampler = if material.point_filter { point } else { linear };
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            entry(1, view(material.base_texture, &defaults.base)),
            entry(2, view(material.normal_texture, &defaults.normal)),
            entry(3, view(material.orm_texture, &defaults.orm)),
            entry(4, view(material.emissive_texture, &defaults.emissive)),
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn put<T>(slots: &mut Vec<Option<T>>, index: u32, value: T) {
    let i = index as usize;
    if slots.len() <= i {
        slots.resize_with(i + 1, || None);
    }
    slots[i] = Some(value);
}

/// Uploads a texture with its CPU mip chain. Levels larger than
/// `max_dimension` are skipped (the texture starts at the first that fits);
/// invalid data becomes 1×1 magenta.
pub(super) fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    data: &TextureData,
    max_dimension: u32,
) -> GpuTexture {
    let data = if data.is_valid() {
        data.to_gpu_layout()
    } else {
        log::warn!(
            "texture {}x{} {:?}: {} bytes don't match; using magenta",
            data.width,
            data.height,
            data.format,
            data.pixels.len()
        );
        TextureData::solid([255, 0, 255, 255], true)
    };
    let chain = data.mip_chain();
    let levels: Vec<&TextureData> = std::iter::once(&data)
        .chain(&chain)
        .skip_while(|l| l.width > max_dimension || l.height > max_dimension)
        .collect();
    let base = levels[0];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("olvin texture"),
        size: wgpu::Extent3d {
            width: base.width,
            height: base.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: data.format.gpu_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip, level) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &level.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(level.width * level.format.bytes_per_pixel() as u32),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: level.width,
                height: level.height,
                depth_or_array_layers: 1,
            },
        );
    }
    GpuTexture {
        view: texture.create_view(&Default::default()),
        mips: levels.len() as u32,
        generation: 0,
    }
}
