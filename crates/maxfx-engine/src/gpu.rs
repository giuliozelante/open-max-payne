//! Hardware renderer (wgpu): static level geometry on the GPU, read back into the same
//! `0x00RRGGBB` frame and view-depth buffers the software rasterizer uses.
//!
//! Shading mirrors `render3d::rasterize_tri`: nearest/repeat texture, alpha test `a >= ref`,
//! `texture × lightmap / 255` (`D3DTOP_MODULATE`), otherwise a normal-based shade.

use crate::render3d::{poly_policy, poly_shade, DETAIL_OFFSET_M, FAR_Z, FRAME_H, FRAME_W, NEAR_Z};
use crate::scene::LdbScene;
use std::collections::BTreeMap;

/// One vertex of a static batch, in world space.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub luv: [f32; 2],
    pub shade: f32,
    /// First vertex of the source polygon (back-face test, as in `draw_poly`).
    pub cull_ref: [f32; 3],
    pub normal: [f32; 3],
}

/// Triangles that share a texture, lightmap and pass.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticBatch {
    pub texture: Option<usize>,
    pub lightmap: Option<usize>,
    pub alpha_reference: Option<u8>,
    pub writes_z: bool,
    pub depth_bias: f32,
    pub verts: Vec<GpuVertex>,
}

/// Eye position, right, up, forward.
pub type GpuView = ([f32; 3], [f32; 3], [f32; 3], [f32; 3]);

/// Device, pipelines and the uploaded static batches of one level.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    opaque: wgpu::RenderPipeline,
    overlay: wgpu::RenderPipeline,
    camera_buf: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    draws: Vec<GpuDraw>,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    color_out: wgpu::Buffer,
    depth_out: wgpu::Buffer,
}

struct GpuDraw {
    writes_z: bool,
    vertices: wgpu::Buffer,
    count: u32,
    group: wgpu::BindGroup,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    pos: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    fwd: [f32; 4],
    /// x = fov / half width, y = fov / half height, z = use lightmaps, w = far plane.
    params: [f32; 4],
    /// x = near plane.
    params2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BatchUniform {
    /// x = alpha reference / 255 (-1 = none), y = depth bias, z = has texture, w = has lightmap.
    params: [f32; 4],
}

const SHADER: &str = r#"
struct Camera { pos: vec4f, right: vec4f, up: vec4f, fwd: vec4f, params: vec4f, params2: vec4f };
struct Batch { params: vec4f };
@group(0) @binding(0) var<uniform> cam: Camera;
@group(1) @binding(0) var<uniform> batch: Batch;
@group(1) @binding(1) var tex: texture_2d<f32>;
@group(1) @binding(2) var lmap: texture_2d<f32>;
@group(1) @binding(3) var tex_sampler: sampler;
@group(1) @binding(4) var lmap_sampler: sampler;

struct In {
    @location(0) pos: vec3f,
    @location(1) uv: vec2f,
    @location(2) luv: vec2f,
    @location(3) shade: f32,
    @location(4) cull_ref: vec3f,
    @location(5) normal: vec3f,
};
struct Out {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) luv: vec2f,
    @location(2) shade: f32,
    @location(3) vz: f32,
};

@vertex
fn vs(v: In) -> Out {
    var o: Out;
    // draw_poly: skip the polygon when its first vertex faces away (dot >= 0.05).
    if dot(v.cull_ref - cam.pos.xyz, v.normal) >= 0.05 {
        o.clip = vec4f(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let d = v.pos - cam.pos.xyz;
    let cx = dot(d, cam.right.xyz);
    let cy = dot(d, cam.up.xyz);
    let cz = dot(d, cam.fwd.xyz);
    // Clip z >= 0 is the software near plane (cz >= NEAR_Z).
    o.clip = vec4f(cx * cam.params.x, cy * cam.params.y, cz - cam.params2.x, cz);
    o.uv = v.uv;
    o.luv = v.luv;
    o.shade = v.shade;
    o.vz = cz;
    return o;
}

struct Frag { @location(0) color: vec4f, @builtin(frag_depth) depth: f32 };

@fragment
fn fs(i: Out) -> Frag {
    var base = vec4f(0.533, 0.6, 0.667, 1.0);
    if batch.params.z > 0.5 {
        base = textureSample(tex, tex_sampler, fract(i.uv));
    }
    if batch.params.x >= 0.0 && base.a < batch.params.x {
        discard;
    }
    var rgb = base.rgb * i.shade;
    if batch.params.w > 0.5 && cam.params.z > 0.5 {
        rgb = base.rgb * textureSample(lmap, lmap_sampler, clamp(i.luv, vec2f(0.0), vec2f(1.0))).rgb;
    }
    var f: Frag;
    f.color = vec4f(rgb, 1.0);
    f.depth = clamp((i.vz - batch.params.y) / cam.params.w, 0.0, 1.0);
    return f;
}
"#;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl GpuRenderer {
    /// `None` when no adapter or device is available.
    pub fn new(scene: &LdbScene) -> Option<Self> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let (w, h) = (FRAME_W as u32, FRAME_H as u32);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("static"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let batch_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("batch"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
                sampler_entry(4),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("static"),
            bind_group_layouts: &[Some(&camera_layout), Some(&batch_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32, 4 => Float32x3, 5 => Float32x3
        ];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let targets = [Some(wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = |writes_z: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("static"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(writes_z),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline(true);
        let overlay = pipeline(false);

        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let sampler = |address| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                ..Default::default()
            })
        };
        let tex_sampler = sampler(wgpu::AddressMode::Repeat);
        let lmap_sampler = sampler(wgpu::AddressMode::ClampToEdge);
        let upload = |width: u32, height: u32, pixels: &[u32]| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let bytes: Vec<u8> = pixels
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8])
                .collect();
            queue.write_texture(
                texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            texture.create_view(&Default::default())
        };
        let blank = upload(1, 1, &[0xffff_ffff]);
        let textures: Vec<_> = scene
            .textures
            .iter()
            .map(|t| (t.width > 0 && t.height > 0).then(|| upload(t.width, t.height, &t.pixels)))
            .collect();
        let lightmaps: Vec<_> = scene
            .lightmaps
            .iter()
            .map(|l| (l.width > 0 && l.height > 0).then(|| upload(l.width, l.height, &l.pixels)))
            .collect();

        let mut draws = Vec::new();
        for batch in build_static_batches(scene) {
            let tex = batch
                .texture
                .and_then(|i| textures.get(i))
                .and_then(Option::as_ref);
            let lmap = batch
                .lightmap
                .and_then(|i| lightmaps.get(i))
                .and_then(Option::as_ref);
            let uniform = BatchUniform {
                params: [
                    batch.alpha_reference.map_or(-1.0, |r| f32::from(r) / 255.0),
                    batch.depth_bias,
                    if tex.is_some() { 1.0 } else { 0.0 },
                    if lmap.is_some() { 1.0 } else { 0.0 },
                ],
            };
            let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: std::mem::size_of::<BatchUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniform));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &batch_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(tex.unwrap_or(&blank)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(lmap.unwrap_or(&blank)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&tex_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&lmap_sampler),
                    },
                ],
            });
            let vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: std::mem::size_of_val(batch.verts.as_slice()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vertices, 0, bytemuck::cast_slice(&batch.verts));
            draws.push(GpuDraw {
                writes_z: batch.writes_z,
                vertices,
                count: batch.verts.len() as u32,
                group,
            });
        }

        let target = |format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let color = target(COLOR_FORMAT, attach);
        let depth = target(DEPTH_FORMAT, attach);
        let readback = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(4 * w * h),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        };
        let color_out = readback();
        let depth_out = readback();
        Some(Self {
            device,
            queue,
            opaque,
            overlay,
            camera_buf,
            camera_group,
            draws,
            color,
            depth,
            color_out,
            depth_out,
        })
    }

    /// Draw the static batches and write covered pixels into `fb` and their view depth into `zb`.
    pub fn render(
        &mut self,
        _scene: &LdbScene,
        (pos, right, up, fwd): GpuView,
        use_lightmaps: bool,
        fb: &mut [u32],
        zb: &mut [f32],
    ) {
        let (w, h) = (FRAME_W as u32, FRAME_H as u32);
        let fov_scale = (FRAME_H as f32) * 0.85;
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let camera = CameraUniform {
            pos: v4(pos),
            right: v4(right),
            up: v4(up),
            fwd: v4(fwd),
            params: [
                fov_scale / (w as f32 * 0.5),
                fov_scale / (h as f32 * 0.5),
                if use_lightmaps { 1.0 } else { 0.0 },
                FAR_Z,
            ],
            params2: [NEAR_Z, 0.0, 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&camera));
        let color_view = self.color.create_view(&Default::default());
        let depth_view = self.depth.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("static"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.camera_group, &[]);
            // Opaque first, then the polygons that test but do not write depth (as render_poly_batch).
            for writes_z in [true, false] {
                pass.set_pipeline(if writes_z {
                    &self.opaque
                } else {
                    &self.overlay
                });
                for draw in self.draws.iter().filter(|d| d.writes_z == writes_z) {
                    pass.set_bind_group(1, &draw.group, &[]);
                    pass.set_vertex_buffer(0, draw.vertices.slice(..));
                    pass.draw(0..draw.count, 0..1);
                }
            }
        }
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * w),
            rows_per_image: Some(h),
        };
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.color_out,
                layout,
            },
            size,
        );
        encoder.copy_texture_to_buffer(
            self.depth.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.depth_out,
                layout,
            },
            size,
        );
        self.queue.submit([encoder.finish()]);
        self.color_out
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        self.depth_out
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        if let (Ok(colors), Ok(depths)) = (
            self.color_out.slice(..).get_mapped_range(),
            self.depth_out.slice(..).get_mapped_range(),
        ) {
            let depths: &[f32] = bytemuck::cast_slice(&depths);
            for (i, (rgba, &d)) in colors.as_chunks::<4>().0.iter().zip(depths).enumerate() {
                if d >= 1.0 {
                    continue;
                }
                fb[i] = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
                zb[i] = d * FAR_Z;
            }
        }
        self.color_out.unmap();
        self.depth_out.unmap();
    }
}

/// Texture, lightmap, alpha reference, writes depth, detail offset.
type BatchKey = (Option<usize>, Option<usize>, Option<u8>, bool, i32);

/// Static room meshes grouped into batches (fan-triangulated, world-space positions).
pub fn build_static_batches(scene: &LdbScene) -> Vec<StaticBatch> {
    let mut batches: Vec<StaticBatch> = Vec::new();
    let mut index: BTreeMap<BatchKey, usize> = BTreeMap::new();
    for sm in &scene.static_meshes {
        let o = sm.world_origin;
        for poly in &sm.polys {
            let policy = poly_policy(scene, poly.material_id);
            if !policy.draw || poly.vertices.len() < 3 {
                continue;
            }
            let material = scene.materials.get(&poly.material_id);
            let texture = material.and_then(|m| m.texture_index);
            let alpha_reference = material.and_then(|m| m.alpha_reference);
            let lightmap = (poly.lightmap_id >= 0).then_some(poly.lightmap_id as usize);
            let key = (
                texture,
                lightmap,
                alpha_reference,
                policy.writes_z,
                policy.detail_offset,
            );
            let at = *index.entry(key).or_insert_with(|| {
                batches.push(StaticBatch {
                    texture,
                    lightmap,
                    alpha_reference,
                    writes_z: policy.writes_z,
                    depth_bias: policy.detail_offset as f32 * DETAIL_OFFSET_M,
                    verts: Vec::new(),
                });
                batches.len() - 1
            });
            let world = |p: [f32; 3]| [p[0] + o[0], p[1] + o[1], p[2] + o[2]];
            let cull_ref = world(poly.vertices[0].pos);
            let shade = poly_shade(poly.normal);
            let gv = |k: usize| {
                let v = &poly.vertices[k];
                GpuVertex {
                    pos: world(v.pos),
                    uv: v.uv,
                    luv: v.luv,
                    shade,
                    cull_ref,
                    normal: poly.normal,
                }
            };
            let out = &mut batches[at].verts;
            for k in 1..poly.vertices.len() - 1 {
                out.extend([gv(0), gv(k), gv(k + 1)]);
            }
        }
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SceneMaterial, ScenePoly, SceneStaticMesh, SceneVertex};

    fn vert(pos: [f32; 3]) -> SceneVertex {
        SceneVertex {
            pos,
            uv: [0.0, 0.0],
            luv: [0.0, 0.0],
        }
    }

    fn quad(material_id: i32, lightmap_id: i32) -> ScenePoly {
        ScenePoly {
            vertices: vec![
                vert([0.0, 0.0, -1.0]),
                vert([0.0, 2.0, -1.0]),
                vert([0.0, 2.0, 1.0]),
                vert([0.0, 0.0, 1.0]),
            ],
            normal: [-1.0, 0.0, 0.0],
            material_id,
            lightmap_id,
        }
    }

    fn scene_with(polys: Vec<ScenePoly>, origin: [f32; 3]) -> LdbScene {
        let mut scene = crate::render3d::test_empty_scene();
        scene.static_meshes.push(SceneStaticMesh {
            mesh_id: 1,
            room_id: None,
            world_origin: origin,
            polys,
        });
        scene
    }

    #[test]
    fn static_polys_with_the_same_material_share_one_fan_triangulated_batch() {
        let scene = scene_with(vec![quad(7, -1), quad(7, -1)], [10.0, 0.0, 0.0]);
        let batches = build_static_batches(&scene);
        assert_eq!(batches.len(), 1);
        // Two quads, two triangles each.
        assert_eq!(batches[0].verts.len(), 12);
        // World origin is baked in, and every vertex keeps its polygon's first vertex for culling.
        assert_eq!(batches[0].verts[0].pos, [10.0, 0.0, -1.0]);
        assert_eq!(batches[0].verts[5].cull_ref, [10.0, 0.0, -1.0]);
        assert_eq!(batches[0].verts[0].normal, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn lightmap_and_draw_policy_split_or_drop_batches() {
        let mut scene = scene_with(vec![quad(7, -1), quad(7, 0), quad(8, -1)], [0.0; 3]);
        // Material 8 is a `dummy` (not drawn, see `material_policy`).
        scene.materials.insert(
            8,
            SceneMaterial {
                id: 8,
                category: "dummy".into(),
                texture_path: String::new(),
                texture_index: None,
                alpha_reference: None,
            },
        );
        let batches = build_static_batches(&scene);
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().any(|b| b.lightmap.is_none()));
        assert!(batches.iter().any(|b| b.lightmap == Some(0)));
    }

    #[test]
    fn gpu_frame_matches_the_software_frame() {
        let mut scene = scene_with(vec![quad(7, -1)], [0.0; 3]);
        scene.static_meshes[0].polys[0].normal = [-1.0, 0.0, 0.0];
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_frame_matches_the_software_frame skipped");
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (sw_fb, sw_zb) = crate::render3d::test_software_static(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &mut fb, &mut zb);
        let covered = |z: &[f32], i: usize| z[i].is_finite();
        let sw_cov = (0..w * h).filter(|&i| covered(&sw_zb, i)).count();
        let agree = (0..w * h)
            .filter(|&i| covered(&sw_zb, i) == covered(&zb, i))
            .count();
        assert!(sw_cov > 1000, "the quad must cover pixels");
        // Edge pixels may differ by rasterisation rules; the rest must match.
        assert!(
            agree as f32 / (w * h) as f32 > 0.99,
            "coverage agreement {agree}"
        );
        let center = (h / 2) * w + w / 2;
        assert_eq!(fb[center], sw_fb[center], "same shade at the centre");
        assert!((zb[center] - sw_zb[center]).abs() < 0.01, "same view depth");
    }
}
