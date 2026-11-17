//! GPU side of the viewport: draws into egui's own render pass through a paint
//! callback, using egui's depth buffer. Buffers are kept per viewport and grow
//! as needed.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, wgpu};

use crate::engine::{EngineGpu, Source};
use crate::ext::{GpuVertex, LineVertex};

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const DEPTH_BITS: u8 = 32;
pub const MSAA: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub light: [f32; 4],
    pub grid: [f32; 4],
}

struct Target {
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    buffers: [(wgpu::Buffer, u64); 3],
    counts: [u32; 3],
    grid: bool,
}

pub struct Resources {
    layout: wgpu::BindGroupLayout,
    mesh: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    targets: HashMap<u64, Target>,
    pub engine: EngineGpu,
}

pub fn init(render_state: &egui_wgpu::RenderState) {
    let device = &render_state.device;
    let format = render_state.target_format;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("olvin viewport"),
        source: wgpu::ShaderSource::Wgsl(include_str!("viewport.wgsl").into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("olvin viewport"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("olvin viewport"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });

    let mesh_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Unorm8x4, 3 => Uint32];
    let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
    let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);

    let pipeline = |label: &str,
                    vs: &str,
                    fs: &str,
                    buffers: &[Option<wgpu::VertexBufferLayout>],
                    topology: wgpu::PrimitiveTopology,
                    depth_write: bool,
                    depth_compare: wgpu::CompareFunction,
                    blend: Option<wgpu::BlendState>| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState { topology, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(depth_write),
                depth_compare: Some(depth_compare),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };

    let mesh_layout = wgpu::VertexBufferLayout {
        array_stride: size_of::<GpuVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &mesh_attrs,
    };
    let line_layout = wgpu::VertexBufferLayout {
        array_stride: size_of::<LineVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &line_attrs,
    };
    use wgpu::CompareFunction::{Always, Less, LessEqual};
    use wgpu::PrimitiveTopology::{LineList, TriangleList};
    let resources = Resources {
        mesh: pipeline("olvin mesh", "vs_mesh", "fs_mesh", &[Some(mesh_layout)], TriangleList, true, Less, None),
        grid: pipeline("olvin grid", "vs_grid", "fs_grid", &[], TriangleList, false, LessEqual, premultiplied),
        lines: pipeline("olvin lines", "vs_line", "fs_line", &[Some(line_layout.clone())], LineList, false, LessEqual, premultiplied),
        overlay: pipeline("olvin overlay", "vs_line", "fs_line", &[Some(line_layout)], LineList, false, Always, premultiplied),
        layout,
        targets: HashMap::new(),
        engine: EngineGpu::new(render_state),
    };
    render_state.renderer.write().callback_resources.insert(resources);
}

/// One viewport's frame, handed to egui as a paint callback.
pub struct Frame {
    pub viewport: u64,
    pub uniforms: Uniforms,
    pub vertices: Vec<GpuVertex>,
    pub lines: Vec<LineVertex>,
    pub overlay: Vec<LineVertex>,
    pub grid: bool,
    /// Engine-rendered world prepared for this viewport id, drawn first.
    pub engine: Option<Source>,
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, slot: &mut (wgpu::Buffer, u64), data: &[u8]) {
    let needed = data.len() as u64;
    if needed > slot.1 {
        let size = needed.next_power_of_two().max(4096);
        slot.0 = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("olvin viewport geometry"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        slot.1 = size;
    }
    if needed > 0 {
        queue.write_buffer(&slot.0, 0, data);
    }
}

impl egui_wgpu::CallbackTrait for Frame {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Resources>() else { return Vec::new() };
        // The engine view was prepared during UI; its shadow map renders here,
        // before egui's pass samples it.
        if let Some(source) = self.engine {
            res.engine.render_shadows(self.viewport, source, encoder);
        }
        let layout = &res.layout;
        let target = res.targets.entry(self.viewport).or_insert_with(|| {
            let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("olvin viewport uniforms"),
                size: size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("olvin viewport"),
                layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
            });
            let empty = || {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("olvin viewport geometry"),
                    size: 4096,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                (buffer, 4096)
            };
            Target { uniforms, bind, buffers: [empty(), empty(), empty()], counts: [0; 3], grid: false }
        });
        queue.write_buffer(&target.uniforms, 0, bytemuck::bytes_of(&self.uniforms));
        upload(device, queue, &mut target.buffers[0], bytemuck::cast_slice(&self.vertices));
        upload(device, queue, &mut target.buffers[1], bytemuck::cast_slice(&self.lines));
        upload(device, queue, &mut target.buffers[2], bytemuck::cast_slice(&self.overlay));
        target.counts = [self.vertices.len() as u32, self.lines.len() as u32, self.overlay.len() as u32];
        target.grid = self.grid;
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(res) = resources.get::<Resources>() else { return };
        if let Some(source) = self.engine {
            res.engine.draw(self.viewport, source, pass);
        }
        let Some(t) = res.targets.get(&self.viewport) else { return };
        pass.set_bind_group(0, &t.bind, &[]);
        if t.counts[0] > 0 {
            pass.set_pipeline(&res.mesh);
            pass.set_vertex_buffer(0, t.buffers[0].0.slice(..));
            pass.draw(0..t.counts[0], 0..1);
        }
        if t.grid {
            pass.set_pipeline(&res.grid);
            pass.draw(0..3, 0..1);
        }
        if t.counts[1] > 0 {
            pass.set_pipeline(&res.lines);
            pass.set_vertex_buffer(0, t.buffers[1].0.slice(..));
            pass.draw(0..t.counts[1], 0..1);
        }
        if t.counts[2] > 0 {
            pass.set_pipeline(&res.overlay);
            pass.set_vertex_buffer(0, t.buffers[2].0.slice(..));
            pass.draw(0..t.counts[2], 0..1);
        }
    }
}
