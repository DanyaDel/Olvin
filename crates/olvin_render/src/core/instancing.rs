//! Instance batching: opaque instances bucketed by (material, mesh), and
//! alpha-blended ones sorted back to front, each packed into one buffer.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::ops::Range;

use crate::pipelines::instance_buffer;

use super::InstanceRaw;

const MIN_INSTANCE_CAPACITY: usize = 1024;

/// Cheap hasher for the (material, mesh) bucket keys.
#[derive(Default)]
struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    fn write_u64(&mut self, v: u64) {
        self.0 = (v ^ (v >> 29)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

pub(super) fn bucket_key(material: u32, mesh: u32) -> u64 {
    ((material as u64) << 32) | mesh as u64
}

/// One instanced draw: sorted by pipeline, then material, then mesh.
pub(super) struct DrawRange {
    pub(super) pipeline: u32,
    pub(super) material: u32,
    pub(super) mesh: u32,
    pub(super) instances: Range<u32>,
}

/// Instances bucketed by (material, mesh), packed into one vertex buffer.
/// Buckets, the key map and the draw list are reused, so a steady frame
/// doesn't allocate.
pub(super) struct InstanceSet {
    pub(super) buffer: wgpu::Buffer,
    capacity: usize,
    buckets: Vec<(u64, Vec<InstanceRaw>)>,
    lookup: HashMap<u64, u32, BuildHasherDefault<KeyHasher>>,
    /// Last key pushed and its bucket (consecutive entities often share it).
    last: Option<(u64, u32)>,
    /// (pipeline, key, bucket) of non-empty buckets, sorted.
    order: Vec<(u32, u64, u32)>,
    pub(super) ranges: Vec<DrawRange>,
    pub(super) count: u32,
}

impl InstanceSet {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: instance_buffer(device, MIN_INSTANCE_CAPACITY),
            capacity: MIN_INSTANCE_CAPACITY,
            buckets: Vec::new(),
            lookup: HashMap::default(),
            last: None,
            order: Vec::new(),
            ranges: Vec::new(),
            count: 0,
        }
    }

    pub(super) fn begin(&mut self) {
        // Forget pairs no longer drawn once they pile up (rare reallocation).
        if self.buckets.len() > 2 * self.ranges.len() + 256 {
            self.buckets.clear();
            self.lookup.clear();
        }
        self.buckets.iter_mut().for_each(|(_, b)| b.clear());
        self.last = None;
    }

    #[inline]
    pub(super) fn push(&mut self, key: u64, instance: InstanceRaw) {
        let index = match self.last {
            Some((k, i)) if k == key => i,
            _ => {
                let next = self.buckets.len() as u32;
                let i = *self.lookup.entry(key).or_insert(next);
                if i == next {
                    self.buckets.push((key, Vec::new()));
                }
                self.last = Some((key, i));
                i
            }
        };
        self.buckets[index as usize].1.push(instance);
    }

    /// Sorts the buckets into draws, packs them through `staging` (reused
    /// scratch) and uploads them.
    pub(super) fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Vec<InstanceRaw>,
        pipeline_of: impl Fn(u32) -> u32,
    ) {
        self.order.clear();
        for (i, (key, bucket)) in self.buckets.iter().enumerate() {
            if !bucket.is_empty() {
                self.order
                    .push((pipeline_of((key >> 32) as u32), *key, i as u32));
            }
        }
        self.order.sort_unstable();
        staging.clear();
        self.ranges.clear();
        for &(pipeline, key, i) in &self.order {
            let start = staging.len() as u32;
            staging.extend_from_slice(&self.buckets[i as usize].1);
            self.ranges.push(DrawRange {
                pipeline,
                material: (key >> 32) as u32,
                mesh: key as u32,
                instances: start..staging.len() as u32,
            });
        }
        write_instances(device, queue, &mut self.buffer, &mut self.capacity, staging);
        self.count = staging.len() as u32;
    }
}

/// One alpha-blended instance waiting to be sorted.
pub(super) struct BlendItem {
    /// View depth of the instance's bounding sphere centre.
    pub(super) depth: f32,
    pub(super) key: u64,
    pub(super) instance: InstanceRaw,
}

/// Alpha-blended instances, drawn back to front. Consecutive instances of
/// the same (material, mesh) share a draw call; items and ranges are reused.
pub(super) struct BlendSet {
    pub(super) buffer: wgpu::Buffer,
    capacity: usize,
    pub(super) items: Vec<BlendItem>,
    pub(super) ranges: Vec<DrawRange>,
    pub(super) count: u32,
}

impl BlendSet {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: instance_buffer(device, MIN_INSTANCE_CAPACITY),
            capacity: MIN_INSTANCE_CAPACITY,
            items: Vec::new(),
            ranges: Vec::new(),
            count: 0,
        }
    }

    /// Sorts far to back (ties by key, so equal depths still batch), packs
    /// the instances through `staging` and uploads them.
    pub(super) fn finish(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        staging: &mut Vec<InstanceRaw>,
        pipeline_of: impl Fn(u32) -> u32,
    ) {
        self.items
            .sort_unstable_by(|a, b| b.depth.total_cmp(&a.depth).then(a.key.cmp(&b.key)));
        staging.clear();
        self.ranges.clear();
        let mut last = None;
        for item in &self.items {
            let i = staging.len() as u32;
            staging.push(item.instance);
            match self.ranges.last_mut() {
                Some(range) if last == Some(item.key) => range.instances.end = i + 1,
                _ => {
                    let material = (item.key >> 32) as u32;
                    self.ranges.push(DrawRange {
                        pipeline: pipeline_of(material),
                        material,
                        mesh: item.key as u32,
                        instances: i..i + 1,
                    });
                    last = Some(item.key);
                }
            }
        }
        write_instances(device, queue, &mut self.buffer, &mut self.capacity, staging);
        self.count = staging.len() as u32;
    }
}

/// Uploads `staging` at the start of `buffer`, growing it (to a power of
/// two) when it doesn't fit.
fn write_instances(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &mut wgpu::Buffer,
    capacity: &mut usize,
    staging: &[InstanceRaw],
) {
    if staging.len() > *capacity {
        *capacity = staging.len().next_power_of_two();
        *buffer = instance_buffer(device, *capacity);
    }
    if !staging.is_empty() {
        queue.write_buffer(buffer, 0, bytemuck::cast_slice(staging));
    }
}
