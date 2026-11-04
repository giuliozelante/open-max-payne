//! Hardware renderer (wgpu): static level geometry on the GPU, read back into the same
//! `0x00RRGGBB` frame and view-depth buffers the software rasterizer uses.
//!
//! Shading mirrors `render3d::rasterize_tri`: nearest/repeat texture, alpha test `a >= ref`,
//! `texture × lightmap / 255` (`D3DTOP_MODULATE`), otherwise a normal-based shade.

use crate::kf2::KfsMesh;
use crate::particles::{Sprite, SpriteTexture};
use crate::render3d::{
    dynamic_mesh_drawn, dynamic_world, poly_policy, DETAIL_OFFSET_M, FAR_Z, FRAME_H, FRAME_W,
    NEAR_Z,
};
use crate::scene::LdbScene;
use std::collections::BTreeMap;

/// One vertex of a static batch, in world space.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub luv: [f32; 2],
    /// First vertex of the source polygon (back-face test, as in `draw_poly`).
    pub cull_ref: [f32; 3],
    pub normal: [f32; 3],
    /// Per-vertex diffuse, used when the model asks for it (characters, #37).
    pub light: [f32; 3],
}

/// Triangles that share a texture, lightmap and pass.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticBatch {
    pub texture: Option<usize>,
    pub lightmap: Option<usize>,
    pub alpha_reference: Option<u8>,
    pub writes_z: bool,
    pub depth_bias: f32,
    /// Dynamic mesh index, `None` for static room geometry.
    pub mesh: Option<usize>,
    /// Room of static geometry (`render3d::static_meshes_in_view`), `None` for dynamic meshes.
    pub room: Option<i32>,
    /// Alpha-blended after the alpha test (`BlendedAlphaTest`).
    pub blended: bool,
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
    /// The same two depth modes, alpha-blended (BlendedAlphaTest batches).
    opaque_blend: wgpu::RenderPipeline,
    overlay_blend: wgpu::RenderPipeline,
    camera_buf: wgpu::Buffer,
    camera_group: wgpu::BindGroup,
    /// `[0]` static geometry (identity), `[1 + i]` dynamic mesh `i`.
    models: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    draws: Vec<GpuDraw>,
    binder: Binder,
    characters: CharacterCache,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    color_out: wgpu::Buffer,
    depth_out: wgpu::Buffer,
    full_shader: wgpu::ShaderModule,
    full_layout: wgpu::PipelineLayout,
    image_layout: wgpu::BindGroupLayout,
    /// Fullscreen passes inside the scene pass: the CPU sky behind, the CPU HUD on top.
    background: wgpu::RenderPipeline,
    hud: wgpu::RenderPipeline,
    background_image: (wgpu::Texture, wgpu::BindGroup),
    overlay_image: (wgpu::Texture, wgpu::BindGroup),
    sky_pipe: wgpu::RenderPipeline,
    /// World sphere image group and whether a sphere is set (else the flat background).
    sky: (wgpu::BindGroup, bool),
    /// Particle sprites: alpha-blended and additive pipelines.
    sprite_alpha: wgpu::RenderPipeline,
    sprite_add: wgpu::RenderPipeline,
    /// Uploaded particle bitmaps.
    sprite_textures: BTreeMap<SpriteTexture, wgpu::BindGroup>,
    sprite_vertices: Option<wgpu::Buffer>,
    /// `(additive, texture, first vertex, vertex count)` for this frame.
    sprite_draws: Vec<(bool, SpriteTexture, u32, u32)>,
}

/// The window's swapchain plus the offscreen colour and depth targets at window size.
pub struct GpuSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    color_group: wgpu::BindGroup,
    blit: wgpu::RenderPipeline,
}

impl GpuSurface {
    fn new(
        renderer: &GpuRenderer,
        surface: wgpu::Surface<'static>,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&renderer.device, &config);
        let entry = if format.is_srgb() {
            "fs_blit_srgb"
        } else {
            "fs_image"
        };
        let blit = fullscreen_pipeline(
            &renderer.device,
            &renderer.full_layout,
            &renderer.full_shader,
            entry,
            format,
            None,
            false,
        );
        let (color, depth, color_group) = window_targets(renderer, config.width, config.height);
        Self {
            surface,
            config,
            color,
            depth,
            color_group,
            blit,
        }
    }

    fn resize(&mut self, renderer: &GpuRenderer, width: u32, height: u32) {
        if (width, height) == (self.config.width, self.config.height) || width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&renderer.device, &self.config);
        (self.color, self.depth, self.color_group) = window_targets(renderer, width, height);
    }
}

fn window_targets(
    renderer: &GpuRenderer,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::Texture, wgpu::BindGroup) {
    let target = |format, usage| {
        renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
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
    let color = target(
        COLOR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let depth = target(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let group = image_group(
        &renderer.device,
        &renderer.image_layout,
        &color.create_view(&Default::default()),
        &renderer.binder.image_sampler,
    );
    (color, depth, group)
}

fn image_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
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
    })
}

/// Fullscreen triangle sampling one image; `in_scene` adds the depth state the scene pass needs.
fn fullscreen_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment: &str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
    in_scene: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fullscreen"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: in_scene.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// `paint_sky_strip` per pixel: the view direction through the pixel centre, then
/// `sky_uv`. Without a sphere, the flat two-tone background of the software path.
const SKY_SHADER: &str = r#"
struct Camera { pos: vec4f, right: vec4f, up: vec4f, fwd: vec4f, params: vec4f, params2: vec4f, fog: vec4f, fog_range: vec4f };
@group(0) @binding(0) var<uniform> cam: Camera;
@group(1) @binding(0) var img: texture_2d<f32>;
@group(1) @binding(1) var img_sampler: sampler;
@fragment
fn fs_sky(v: V) -> @location(0) vec4f {
    if cam.params2.y < 0.5 {
        if v.uv.y < 0.5 {
            return vec4f(vec3f(22.0, 28.0, 40.0) / 255.0, 1.0);
        }
        return vec4f(vec3f(30.0, 30.0, 34.0) / 255.0, 1.0);
    }
    let w = cam.params2.z;
    let h = cam.params2.w;
    let fov = cam.params.x * w * 0.5;
    let cx = (v.uv.x * w - w * 0.5) / fov;
    let cy = (h * 0.5 - v.uv.y * h) / fov;
    let d = normalize(cam.right.xyz * cx + cam.up.xyz * cy + cam.fwd.xyz);
    let u = fract(atan2(d.x, d.z) / 6.2831853 + 0.5);
    let t = clamp(0.5 - asin(clamp(d.y, -1.0, 1.0)) / 3.14159265, 0.0, 0.999);
    return textureSampleLevel(img, img_sampler, vec2f(u, t), 0.0);
}
"#;

/// Particle sprites (`render3d::draw_sprites`): the bitmap times the vertex colour, depth-tested
/// with the same linear `frag_depth` as the scene shader, never writing depth.
const SPRITE_SHADER: &str = r#"
struct Camera { pos: vec4f, right: vec4f, up: vec4f, fwd: vec4f, params: vec4f, params2: vec4f, fog: vec4f, fog_range: vec4f };
@group(0) @binding(0) var<uniform> cam: Camera;
@group(1) @binding(0) var img: texture_2d<f32>;
@group(1) @binding(1) var img_sampler: sampler;
struct SpriteIn { @location(0) pos: vec3f, @location(1) uv: vec2f, @location(2) color: vec4f };
struct SpriteOut { @builtin(position) clip: vec4f, @location(0) uv: vec2f, @location(1) color: vec4f, @location(2) vz: f32 };
@vertex
fn vs_sprite(v: SpriteIn) -> SpriteOut {
    var o: SpriteOut;
    let d = v.pos - cam.pos.xyz;
    let cz = dot(d, cam.fwd.xyz);
    o.clip = vec4f(dot(d, cam.right.xyz) * cam.params.x, dot(d, cam.up.xyz) * cam.params.y, cz - cam.params2.x, cz);
    o.uv = v.uv;
    o.color = v.color;
    o.vz = cz;
    return o;
}
struct SpriteFrag { @location(0) color: vec4f, @builtin(frag_depth) depth: f32 };
@fragment
fn fs_sprite(i: SpriteOut) -> SpriteFrag {
    let t = textureSample(img, img_sampler, i.uv);
    var f: SpriteFrag;
    f.color = vec4f(t.rgb * i.color.rgb, t.a * i.color.a);
    f.depth = clamp(i.vz / cam.params.w, 0.0, 1.0);
    return f;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteVertex {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

/// Fullscreen triangle vertex stage, shared by the image and sky shaders.
const FULLSCREEN_VS: &str = r#"
struct V { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> V {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: V;
    o.pos = vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2f(x, y);
    return o;
}
"#;

const FULLSCREEN_SHADER: &str = r#"
@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var img_sampler: sampler;
@fragment
fn fs_image(v: V) -> @location(0) vec4f {
    return textureSample(img, img_sampler, v.uv);
}
// The frame holds display values; an sRGB swapchain re-encodes them, so decode first.
@fragment
fn fs_blit_srgb(v: V) -> @location(0) vec4f {
    let c = textureSample(img, img_sampler, v.uv);
    return vec4f(pow(c.rgb, vec3f(2.2)), 1.0);
}
"#;

/// Layouts, samplers and the 1x1 white fallback texture, kept to create bind groups later.
struct Binder {
    batch_layout: wgpu::BindGroupLayout,
    model_layout: wgpu::BindGroupLayout,
    tex_sampler: wgpu::Sampler,
    lmap_sampler: wgpu::Sampler,
    /// Nearest, for screen-space images (frame layers, render targets) and the sky strip.
    image_sampler: wgpu::Sampler,
    blank: wgpu::TextureView,
}

impl Binder {
    fn batch_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniform: BatchUniform,
        tex: Option<&wgpu::TextureView>,
        lmap: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<BatchUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.batch_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(tex.unwrap_or(&self.blank)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(lmap.unwrap_or(&self.blank)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.tex_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.lmap_sampler),
                },
            ],
        })
    }

    fn model(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uniform: ModelUniform,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<ModelUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.model_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        (buffer, group)
    }
}

/// Character textures per mesh (by address, stable for a walkthrough), the per-frame vertex
/// buffer and one model per character shade.
#[derive(Default)]
struct CharacterCache {
    materials: BTreeMap<usize, Vec<wgpu::BindGroup>>,
    model: Option<(wgpu::Buffer, wgpu::BindGroup)>,
    vertices: Option<(wgpu::Buffer, u64)>,
}

/// One posed character for this frame: world-space vertices from `character_world_verts`.
pub struct GpuCharacter<'a> {
    pub mesh: &'a KfsMesh,
    pub verts: Vec<[f32; 3]>,
    /// Lying dead: drawn two-sided (`character_face_normal`).
    pub dead: bool,
    /// Per-vertex diffuse from `character_vertex_light`.
    pub light: Vec<[f32; 3]>,
}

/// Box-filtered mip levels, from the full image down to 1x1 (`(width, height, pixels)`).
fn mip_chain(width: u32, height: u32, pixels: &[u32]) -> Vec<(u32, u32, Vec<u32>)> {
    let mut levels = vec![(width, height, pixels.to_vec())];
    while let Some((w, h, src)) = levels.last().filter(|l| l.0 > 1 || l.1 > 1) {
        let (w, h) = (*w, *h);
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let texel = |x: u32, y: u32| src[(y.min(h - 1) * w + x.min(w - 1)) as usize];
        let mut out = Vec::with_capacity((nw * nh) as usize);
        for y in 0..nh {
            for x in 0..nw {
                let quad = [
                    texel(2 * x, 2 * y),
                    texel(2 * x + 1, 2 * y),
                    texel(2 * x, 2 * y + 1),
                    texel(2 * x + 1, 2 * y + 1),
                ];
                let channel = |shift: u32| {
                    let sum: u32 = quad.iter().map(|p| (p >> shift) & 0xff).sum();
                    ((sum + 2) / 4) << shift
                };
                out.push(channel(24) | channel(16) | channel(8) | channel(0));
            }
        }
        levels.push((nw, nh, out));
    }
    levels
}

/// Upload an image; `mipmaps` adds the `mip_chain` levels (diffuse textures, sampled trilinearly).
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u32],
    mipmaps: bool,
) -> wgpu::TextureView {
    let levels = if mipmaps {
        mip_chain(width, height, pixels)
    } else {
        vec![(width, height, pixels.to_vec())]
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (mip_level, (w, h, level)) in levels.iter().enumerate() {
        let bytes: Vec<u8> = level
            .iter()
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8])
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip_level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&Default::default())
}

struct GpuDraw {
    mesh: Option<usize>,
    room: Option<i32>,
    writes_z: bool,
    blended: bool,
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
    /// x = near plane, y = has sky, z/w = frame width/height (sky directions).
    params2: [f32; 4],
    /// Fog colour (0..1) in xyz, w = fog on; `render3d::apply_fog` in the fragment stage.
    fog: [f32; 4],
    /// x = fog start, y = fog end (view depth).
    fog_range: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ModelUniform {
    c0: [f32; 4],
    c1: [f32; 4],
    c2: [f32; 4],
    origin: [f32; 4],
    flags: [f32; 4],
}

impl ModelUniform {
    /// `mat` uses the scene's column layout (`xform_point`: column `k` is `mat[3k..3k+3]`).
    fn new(mat: [f32; 9], origin: [f32; 3], cull: bool) -> Self {
        Self {
            c0: [mat[0], mat[1], mat[2], 0.0],
            c1: [mat[3], mat[4], mat[5], 0.0],
            c2: [mat[6], mat[7], mat[8], 0.0],
            origin: [origin[0], origin[1], origin[2], 0.0],
            flags: [if cull { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
        }
    }

    /// Identity placement, back-face cull, per-vertex light (characters).
    fn vertex_lit() -> Self {
        let mut m = Self::new(
            [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            [0.0; 3],
            true,
        );
        m.flags[1] = 1.0;
        m
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BatchUniform {
    /// x = alpha reference / 255 (-1 = none), y = depth bias, z = has texture, w = has lightmap.
    params: [f32; 4],
    /// x = blended (texel alpha goes to the blend stage).
    params2: [f32; 4],
}

const SHADER: &str = r#"
struct Camera { pos: vec4f, right: vec4f, up: vec4f, fwd: vec4f, params: vec4f, params2: vec4f, fog: vec4f, fog_range: vec4f };
struct Batch { params: vec4f, params2: vec4f };
@group(0) @binding(0) var<uniform> cam: Camera;
@group(1) @binding(0) var<uniform> batch: Batch;
@group(1) @binding(1) var tex: texture_2d<f32>;
@group(1) @binding(2) var lmap: texture_2d<f32>;
@group(1) @binding(3) var tex_sampler: sampler;
@group(1) @binding(4) var lmap_sampler: sampler;
// Mesh-to-world transform (columns), origin; flags.x = back-face cull.
struct Model { c0: vec4f, c1: vec4f, c2: vec4f, origin: vec4f, flags: vec4f };
@group(2) @binding(0) var<uniform> model: Model;

struct In {
    @location(0) pos: vec3f,
    @location(1) uv: vec2f,
    @location(2) luv: vec2f,
    @location(3) cull_ref: vec3f,
    @location(4) normal: vec3f,
    @location(5) light: vec3f,
};
struct Out {
    @builtin(position) clip: vec4f,
    @location(0) uv: vec2f,
    @location(1) luv: vec2f,
    @location(2) shade: vec3f,
    @location(3) vz: f32,
};

@vertex
fn vs(v: In) -> Out {
    var o: Out;
    let m = mat3x3f(model.c0.xyz, model.c1.xyz, model.c2.xyz);
    let world = m * v.pos + model.origin.xyz;
    let n = m * v.normal;
    // draw_poly: skip the polygon when its first vertex faces away (dot >= 0.05).
    let cull_ref = m * v.cull_ref + model.origin.xyz;
    if model.flags.x > 0.5 && dot(cull_ref - cam.pos.xyz, n) >= 0.05 {
        o.clip = vec4f(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let d = world - cam.pos.xyz;
    let cx = dot(d, cam.right.xyz);
    let cy = dot(d, cam.up.xyz);
    let cz = dot(d, cam.fwd.xyz);
    // Clip z >= 0 is the software near plane (cz >= NEAR_Z).
    o.clip = vec4f(cx * cam.params.x, cy * cam.params.y, cz - cam.params2.x, cz);
    o.uv = v.uv;
    o.luv = v.luv;
    // render3d::poly_shade on the world normal, or the per-vertex light (characters).
    o.shade = vec3f(clamp(abs(n.x * 0.35 + n.y * 0.75 + n.z * 0.55) * 0.55 + 0.45, 0.35, 1.0));
    if model.flags.y > 0.5 {
        o.shade = v.light;
    }
    o.vz = cz;
    return o;
}

struct Frag { @location(0) color: vec4f, @builtin(frag_depth) depth: f32 };

@fragment
fn fs(i: Out) -> Frag {
    var base = vec4f(0.533, 0.6, 0.667, 1.0);
    if batch.params.z > 0.5 {
        base = textureSample(tex, tex_sampler, i.uv);
    }
    if batch.params.x >= 0.0 && base.a < batch.params.x {
        discard;
    }
    var rgb = base.rgb * i.shade;
    if batch.params.w > 0.5 && cam.params.z > 0.5 {
        rgb = base.rgb * textureSample(lmap, lmap_sampler, clamp(i.luv, vec2f(0.0), vec2f(1.0))).rgb;
    }
    if cam.fog.w > 0.5 {
        // render3d::fog_amount: linear in view depth, a step when end <= start.
        let span = cam.fog_range.y - cam.fog_range.x;
        let k = select(step(cam.fog_range.y, i.vz), clamp((i.vz - cam.fog_range.x) / span, 0.0, 1.0), span > 0.0);
        rgb = mix(rgb, cam.fog.rgb, k);
    }
    var f: Frag;
    // Blended batches (BlendedAlphaTest) hand the texel alpha to the blend stage.
    f.color = vec4f(rgb, select(1.0, base.a, batch.params2.x > 0.5));
    f.depth = clamp((i.vz - batch.params.y) / cam.params.w, 0.0, 1.0);
    return f;
}
"#;

/// `0x00RRGGBB` frame pixels as RGBA bytes; pixels equal to `transparent` get alpha 0.
fn frame_rgba(pixels: &[u32], transparent: Option<u32>) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|&p| {
            let a = if Some(p) == transparent { 0 } else { 255 };
            [(p >> 16) as u8, (p >> 8) as u8, p as u8, a]
        })
        .collect()
}

/// The HUD layer with the screen fade on it: clear pixels carry the fade colour at its alpha
/// (alpha-blended over the scene), drawn HUD pixels get the fade mixed in and stay opaque.
fn overlay_rgba(hud: &[u32], fade: Option<[u8; 4]>) -> Vec<u8> {
    let Some(fade) = fade else {
        return frame_rgba(hud, Some(HUD_CLEAR));
    };
    let mut faded = hud.to_vec();
    crate::render3d::blend_screen(&mut faded, fade);
    hud.iter()
        .zip(&faded)
        .flat_map(|(&p, &f)| {
            if p == HUD_CLEAR {
                fade
            } else {
                [(f >> 16) as u8, (f >> 8) as u8, f as u8, 255]
            }
        })
        .collect()
}

/// HUD pixels with this value are transparent in [`GpuRenderer::present`].
pub const HUD_CLEAR: u32 = 0xff00_0000;

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
        Some(Self::build(device, queue, scene))
    }

    /// A renderer and swapchain for `frontend`'s window. `None` when no adapter, device or
    /// surface is available.
    pub fn for_window(
        scene: &LdbScene,
        frontend: &crate::frontend::WinitFrontend,
    ) -> Option<(Self, GpuSurface)> {
        let display = frontend.display_handle()?;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(display),
        ));
        let surface = instance.create_surface(frontend.window.clone()).ok()?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB format: the frame's values are already display values.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())?;
        let renderer = Self::build(device, queue, scene);
        let (w, h) = frontend.size();
        let surface = GpuSurface::new(&renderer, surface, format, w, h);
        Some((renderer, surface))
    }

    /// The same device (and so the same window swapchain) with batches for another scene.
    pub fn for_scene(&self, scene: &LdbScene) -> Self {
        Self::build(self.device.clone(), self.queue.clone(), scene)
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue, scene: &LdbScene) -> Self {
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
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("static"),
            bind_group_layouts: &[
                Some(&camera_layout),
                Some(&batch_layout),
                Some(&model_layout),
            ],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x2, 2 => Float32x2, 3 => Float32x3, 4 => Float32x3, 5 => Float32x3
        ];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let pipeline = |writes_z: bool, blended: bool| {
            let targets = [Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: blended.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })];
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
        let opaque = pipeline(true, false);
        let overlay = pipeline(false, false);
        let opaque_blend = pipeline(true, true);
        let overlay_blend = pipeline(false, true);

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

        let sampler = |address, filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: match filter {
                    wgpu::FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
                    wgpu::FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
                },
                ..Default::default()
            })
        };
        // Retail filters diffuse textures (`FilteringMode`: bilinear, trilinear or anisotropic; the
        // default is not read yet). Trilinear over `mip_chain` levels.
        let tex_sampler = sampler(wgpu::AddressMode::Repeat, wgpu::FilterMode::Linear);
        // `render3d::sample_lightmap`: lightmaps are filtered, so their shading is smooth.
        let lmap_sampler = sampler(wgpu::AddressMode::ClampToEdge, wgpu::FilterMode::Linear);
        let binder = Binder {
            batch_layout,
            model_layout,
            tex_sampler,
            lmap_sampler,
            image_sampler: sampler(wgpu::AddressMode::Repeat, wgpu::FilterMode::Nearest),
            blank: upload(&device, &queue, 1, 1, &[0xffff_ffff], false),
        };
        let textures: Vec<_> = scene
            .textures
            .iter()
            .map(|t| {
                (t.width > 0 && t.height > 0)
                    .then(|| upload(&device, &queue, t.width, t.height, &t.pixels, true))
            })
            .collect();
        let lightmaps: Vec<_> = scene
            .lightmaps
            .iter()
            .map(|l| {
                (l.width > 0 && l.height > 0)
                    .then(|| upload(&device, &queue, l.width, l.height, &l.pixels, false))
            })
            .collect();

        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let mut models =
            vec![binder.model(&device, &queue, ModelUniform::new(identity, [0.0; 3], true))];
        models.extend((0..scene.dynamic_meshes.len()).map(|_| {
            binder.model(
                &device,
                &queue,
                ModelUniform::new(identity, [0.0; 3], false),
            )
        }));

        let mut draws = Vec::new();
        for batch in build_static_batches(scene)
            .into_iter()
            .chain(build_dynamic_batches(scene))
        {
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
                params2: [if batch.blended { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            };
            let group = binder.batch_group(&device, &queue, uniform, tex, lmap);
            let vertices = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: std::mem::size_of_val(batch.verts.as_slice()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&vertices, 0, bytemuck::cast_slice(&batch.verts));
            draws.push(GpuDraw {
                mesh: batch.mesh,
                room: batch.room,
                writes_z: batch.writes_z,
                blended: batch.blended,
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

        let full_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fullscreen"),
            source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_VS}{FULLSCREEN_SHADER}").into()),
        });
        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("image"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let full_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fullscreen"),
            bind_group_layouts: &[Some(&image_layout)],
            immediate_size: 0,
        });
        let fullscreen = |blend| {
            fullscreen_pipeline(
                &device,
                &full_layout,
                &full_shader,
                "fs_image",
                COLOR_FORMAT,
                blend,
                true,
            )
        };
        let background = fullscreen(None);
        let overlay_pipe = fullscreen(Some(wgpu::BlendState::ALPHA_BLENDING));
        let frame_image = || {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let group = image_group(&device, &image_layout, &view, &binder.image_sampler);
            (texture, group)
        };
        let background_image = frame_image();
        let overlay_image = frame_image();
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sky"),
            source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_VS}{SKY_SHADER}").into()),
        });
        let sky_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky"),
            bind_group_layouts: &[Some(&camera_layout), Some(&image_layout)],
            immediate_size: 0,
        });
        let sky_pipe = fullscreen_pipeline(
            &device,
            &sky_layout,
            &sky_shader,
            "fs_sky",
            COLOR_FORMAT,
            None,
            true,
        );
        let sky_group = image_group(&device, &image_layout, &binder.blank, &binder.image_sampler);
        let sprite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite"),
            source: wgpu::ShaderSource::Wgsl(SPRITE_SHADER.into()),
        });
        let sprite_attributes =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let sprite_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SpriteVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &sprite_attributes,
        })];
        let sprite_pipeline = |blend: wgpu::BlendState| {
            let targets = [Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("sprite"),
                layout: Some(&sky_layout),
                vertex: wgpu::VertexState {
                    module: &sprite_shader,
                    entry_point: Some("vs_sprite"),
                    compilation_options: Default::default(),
                    buffers: &sprite_buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &sprite_shader,
                    entry_point: Some("fs_sprite"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sprite_alpha = sprite_pipeline(wgpu::BlendState::ALPHA_BLENDING);
        let sprite_add = sprite_pipeline(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            // Accumulated so `render` sees where additive sprites drew.
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        Self {
            device,
            queue,
            opaque,
            overlay,
            opaque_blend,
            overlay_blend,
            camera_buf,
            camera_group,
            models,
            draws,
            binder,
            characters: CharacterCache::default(),
            color,
            depth,
            color_out,
            depth_out,
            full_shader,
            full_layout,
            image_layout,
            background,
            hud: overlay_pipe,
            background_image,
            overlay_image,
            sky_pipe,
            sky: (sky_group, false),
            sprite_alpha,
            sprite_add,
            sprite_textures: BTreeMap::new(),
            sprite_vertices: None,
            sprite_draws: Vec::new(),
        }
    }

    /// Particle sprites for the next frames: camera-facing squares for `view`, bitmaps from
    /// `texture` (uploaded once per key). Sprites without a bitmap are skipped.
    pub fn set_sprites<'a>(
        &mut self,
        sprites: &[Sprite],
        (_, right, up, _): GpuView,
        texture: impl Fn(&SpriteTexture) -> Option<&'a crate::scene::DecodedTexture>,
    ) {
        let mut order: Vec<&Sprite> = sprites.iter().collect();
        order.sort_by(|a, b| (a.additive, &a.texture).cmp(&(b.additive, &b.texture)));
        let mut verts: Vec<SpriteVertex> = Vec::with_capacity(order.len() * 6);
        self.sprite_draws.clear();
        for sprite in order {
            if !self.sprite_textures.contains_key(&sprite.texture) {
                let Some(t) = texture(&sprite.texture).filter(|t| t.width > 0 && t.height > 0)
                else {
                    continue;
                };
                let view = upload(
                    &self.device,
                    &self.queue,
                    t.width,
                    t.height,
                    &t.pixels,
                    true,
                );
                let group = image_group(
                    &self.device,
                    &self.image_layout,
                    &view,
                    &self.binder.tex_sampler,
                );
                self.sprite_textures.insert(sprite.texture.clone(), group);
            }
            let h = sprite.size * 0.5;
            let corner = |a: f32, b: f32, uv: [f32; 2]| SpriteVertex {
                pos: [0, 1, 2].map(|k| sprite.pos[k] + (right[k] * a + up[k] * b) * h),
                uv,
                color: sprite.rgba,
            };
            let (tl, tr) = (corner(-1.0, 1.0, [0.0, 0.0]), corner(1.0, 1.0, [1.0, 0.0]));
            let (bl, br) = (
                corner(-1.0, -1.0, [0.0, 1.0]),
                corner(1.0, -1.0, [1.0, 1.0]),
            );
            let start = verts.len() as u32;
            verts.extend([tl, tr, br, tl, br, bl]);
            match self.sprite_draws.last_mut() {
                Some((additive, key, _, count))
                    if *additive == sprite.additive && *key == sprite.texture =>
                {
                    *count += 6;
                }
                _ => self
                    .sprite_draws
                    .push((sprite.additive, sprite.texture.clone(), start, 6)),
            }
        }
        self.sprite_vertices = (!verts.is_empty()).then(|| {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sprites"),
                size: std::mem::size_of_val(verts.as_slice()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue
                .write_buffer(&buffer, 0, bytemuck::cast_slice(&verts));
            buffer
        });
    }

    /// Draw the static batches and write covered pixels into `fb` and their view depth into `zb`.
    pub fn render(
        &mut self,
        scene: &LdbScene,
        (pos, right, up, fwd): GpuView,
        use_lightmaps: bool,
        characters: &[GpuCharacter],
        fb: &mut [u32],
        zb: &mut [f32],
    ) {
        let (w, h) = (FRAME_W as u32, FRAME_H as u32);
        let frame = self.prepare_scene(scene, (pos, right, up, fwd), use_lightmaps, characters);
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
            self.draw_scene(&mut pass, &frame);
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
                // Drawn pixels: geometry wrote depth; sprites write no depth but leave alpha
                // over the transparent clear.
                if d >= 1.0 && rgba[3] == 0 {
                    continue;
                }
                fb[i] = (u32::from(rgba[0]) << 16) | (u32::from(rgba[1]) << 8) | u32::from(rgba[2]);
                if d < 1.0 {
                    zb[i] = d * FAR_Z;
                }
            }
        }
        self.color_out.unmap();
        self.depth_out.unmap();
    }
}

/// Dynamic meshes grouped into batches per mesh, in mesh-local space.
pub fn build_dynamic_batches(scene: &LdbScene) -> Vec<StaticBatch> {
    let mut batches = Vec::new();
    for (idx, dm) in scene.dynamic_meshes.iter().enumerate() {
        add_polys(scene, &dm.polys, [0.0; 3], Some(idx), None, &mut batches);
    }
    batches
}

struct CharacterDraw {
    mesh: usize,
    material: usize,
    start: u32,
    count: u32,
}

/// Per-frame scene state: which dynamic meshes draw, and the character draws.
struct SceneFrame {
    visible: Vec<bool>,
    /// Rooms whose static batches are drawn (`render3d::static_meshes_in_view`).
    rooms_in_view: std::collections::BTreeSet<i32>,
    characters: Vec<CharacterDraw>,
}

impl GpuRenderer {
    /// Camera, dynamic mesh transforms and character triangles for this frame.
    fn prepare_scene(
        &mut self,
        scene: &LdbScene,
        (pos, right, up, fwd): GpuView,
        use_lightmaps: bool,
        characters: &[GpuCharacter],
    ) -> SceneFrame {
        let character_draws = self.prepare_characters(characters);
        let (w, h) = (FRAME_W as f32, FRAME_H as f32);
        let fov_scale = crate::render3d::fov_scale(scene.fov_multiplier.value);
        let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
        let camera = CameraUniform {
            pos: v4(pos),
            right: v4(right),
            up: v4(up),
            fwd: v4(fwd),
            params: [
                fov_scale / (w * 0.5),
                fov_scale / (h * 0.5),
                if use_lightmaps { 1.0 } else { 0.0 },
                FAR_Z,
            ],
            params2: [NEAR_Z, if self.sky.1 { 1.0 } else { 0.0 }, w, h],
            fog: scene.fog.map_or([0.0; 4], |fog| {
                [
                    fog.color[0] / 255.0,
                    fog.color[1] / 255.0,
                    fog.color[2] / 255.0,
                    1.0,
                ]
            }),
            fog_range: scene
                .fog
                .map_or([0.0; 4], |fog| [fog.start, fog.end, 0.0, 0.0]),
        };
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&camera));
        let rooms: BTreeMap<i32, [f32; 3]> =
            scene.rooms.iter().map(|r| (r.id, r.world_origin)).collect();
        let visible: Vec<bool> = scene
            .dynamic_meshes
            .iter()
            .map(dynamic_mesh_drawn)
            .collect();
        for (idx, drawn) in visible.iter().enumerate() {
            if let (true, Some((buffer, _))) = (*drawn, self.models.get(idx + 1)) {
                let (mat, origin) = dynamic_world(scene, idx, &rooms);
                let uniform = ModelUniform::new(mat, origin, false);
                self.queue
                    .write_buffer(buffer, 0, bytemuck::bytes_of(&uniform));
            }
        }
        let rooms_in_view = crate::render3d::static_meshes_in_view(scene, scene.attached_camera)
            .filter_map(|sm| sm.room_id)
            .collect();
        SceneFrame {
            visible,
            rooms_in_view,
            characters: character_draws,
        }
    }

    /// Opaque geometry and characters, then the polygons that test but do not write depth
    /// (the order of render_poly_batch).
    fn draw_scene(&self, pass: &mut wgpu::RenderPass, frame: &SceneFrame) {
        pass.set_bind_group(0, &self.camera_group, &[]);
        for (writes_z, blended) in [(true, false), (true, true), (false, false), (false, true)] {
            pass.set_pipeline(match (writes_z, blended) {
                (true, false) => &self.opaque,
                (true, true) => &self.opaque_blend,
                (false, false) => &self.overlay,
                (false, true) => &self.overlay_blend,
            });
            for draw in self
                .draws
                .iter()
                .filter(|d| d.writes_z == writes_z && d.blended == blended)
            {
                if draw.room.is_some_and(|r| !frame.rooms_in_view.contains(&r)) {
                    continue;
                }
                let model = match draw.mesh {
                    Some(i) if !frame.visible.get(i).copied().unwrap_or(false) => continue,
                    Some(i) => i + 1,
                    None => 0,
                };
                pass.set_bind_group(1, &draw.group, &[]);
                pass.set_bind_group(2, &self.models[model].1, &[]);
                pass.set_vertex_buffer(0, draw.vertices.slice(..));
                pass.draw(0..draw.count, 0..1);
            }
            // Characters write depth like the software path (rasterize_tri with write_z).
            if let (true, false, Some((buffer, _))) =
                (writes_z, blended, self.characters.vertices.as_ref())
            {
                pass.set_vertex_buffer(0, buffer.slice(..));
                for d in &frame.characters {
                    pass.set_bind_group(1, &self.characters.materials[&d.mesh][d.material], &[]);
                    if let Some((_, model)) = &self.characters.model {
                        pass.set_bind_group(2, model, &[]);
                    }
                    pass.draw(d.start..d.start + d.count, 0..1);
                }
            }
        }
        if let Some(buffer) = &self.sprite_vertices {
            pass.set_vertex_buffer(0, buffer.slice(..));
            for (additive, key, start, count) in &self.sprite_draws {
                pass.set_pipeline(if *additive {
                    &self.sprite_add
                } else {
                    &self.sprite_alpha
                });
                pass.set_bind_group(1, &self.sprite_textures[key], &[]);
                pass.draw(*start..*start + *count, 0..1);
            }
        }
    }

    /// One window frame: `background` (CPU pixels, e.g. wireframe) or the sky behind, the 3D
    /// scene when `view` is given,
    /// `hud` on top (pixels equal to [`HUD_CLEAR`] are transparent), then a blit to the
    /// swapchain at window size.
    #[allow(clippy::too_many_arguments)]
    pub fn present(
        &mut self,
        surface: &mut GpuSurface,
        window_size: (u32, u32),
        scene: &LdbScene,
        view: Option<GpuView>,
        use_lightmaps: bool,
        characters: &[GpuCharacter],
        background: Option<&[u32]>,
        hud: &[u32],
    ) {
        surface.resize(self, window_size.0, window_size.1);
        let frame = view.map(|v| self.prepare_scene(scene, v, use_lightmaps, characters));
        self.upload_layers(background, hud, scene.screen_fade.map(|fade| fade.color()));
        let output = match surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            _ => {
                surface.surface.configure(&self.device, &surface.config);
                return;
            }
        };
        let color_view = surface.color.create_view(&Default::default());
        let depth_view = surface.depth.create_view(&Default::default());
        let out_view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode_frame(
            &mut encoder,
            &color_view,
            &depth_view,
            frame.as_ref(),
            background.is_some(),
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &out_view,
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
            pass.set_pipeline(&surface.blit);
            pass.set_bind_group(0, &surface.color_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
    }

    /// Upload this frame's character triangles, creating texture groups and shade models on
    /// first use. Returns the draws in buffer order.
    fn prepare_characters(&mut self, characters: &[GpuCharacter]) -> Vec<CharacterDraw> {
        let mut verts: Vec<GpuVertex> = Vec::new();
        let mut draws = Vec::new();
        for ch in characters {
            let key = ch.mesh as *const KfsMesh as usize;
            if !self.characters.materials.contains_key(&key) {
                let groups = (0..ch.mesh.material_textures.len().max(1))
                    .map(|m| {
                        let tex = ch.mesh.material_textures.get(m).and_then(Option::as_ref);
                        let view = tex.filter(|t| t.width > 0 && t.height > 0).map(|t| {
                            upload(
                                &self.device,
                                &self.queue,
                                t.width,
                                t.height,
                                &t.pixels,
                                true,
                            )
                        });
                        let uniform = BatchUniform {
                            params: [-1.0, 0.0, if view.is_some() { 1.0 } else { 0.0 }, 0.0],
                            params2: [0.0; 4],
                        };
                        self.binder.batch_group(
                            &self.device,
                            &self.queue,
                            uniform,
                            view.as_ref(),
                            None,
                        )
                    })
                    .collect();
                self.characters.materials.insert(key, groups);
            }
            if self.characters.model.is_none() {
                let model =
                    self.binder
                        .model(&self.device, &self.queue, ModelUniform::vertex_lit());
                self.characters.model = Some(model);
            }
            let materials = self.characters.materials[&key].len();
            let mut by_material: BTreeMap<usize, Vec<GpuVertex>> = BTreeMap::new();
            for face in &ch.mesh.faces {
                let material = face.mat_index.min(materials - 1);
                let out = by_material.entry(material).or_default();
                let normal =
                    crate::render3d::character_face_normal(&ch.verts, face.indices, ch.dead);
                let cull_ref = ch.verts.get(face.indices[0]).copied().unwrap_or([0.0; 3]);
                for (k, &i) in face.indices.iter().enumerate() {
                    let Some(&pos) = ch.verts.get(i) else {
                        continue;
                    };
                    out.push(GpuVertex {
                        pos,
                        uv: face.uvs[k],
                        luv: [0.5, 0.5],
                        cull_ref,
                        normal,
                        light: ch.light.get(i).copied().unwrap_or([1.0; 3]),
                    });
                }
            }
            for (material, tri) in by_material {
                draws.push(CharacterDraw {
                    mesh: key,
                    material,
                    start: verts.len() as u32,
                    count: tri.len() as u32,
                });
                verts.extend(tri);
            }
        }
        if verts.is_empty() {
            return Vec::new();
        }
        let bytes = std::mem::size_of_val(verts.as_slice()) as u64;
        if self
            .characters
            .vertices
            .as_ref()
            .is_none_or(|(_, cap)| *cap < bytes)
        {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("characters"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.characters.vertices = Some((buffer, bytes.next_power_of_two()));
        }
        if let Some((buffer, _)) = &self.characters.vertices {
            self.queue
                .write_buffer(buffer, 0, bytemuck::cast_slice(&verts));
        }
        draws
    }
}

impl GpuRenderer {
    /// The level's baked world sphere (`sky_uv`), or `None` for the flat background.
    pub fn set_sky(&mut self, sky: Option<&crate::scene::DecodedTexture>) {
        let view = sky.filter(|t| t.width > 0 && t.height > 0).map(|t| {
            upload(
                &self.device,
                &self.queue,
                t.width,
                t.height,
                &t.pixels,
                false,
            )
        });
        let group = image_group(
            &self.device,
            &self.image_layout,
            view.as_ref().unwrap_or(&self.binder.blank),
            &self.binder.image_sampler,
        );
        self.sky = (group, view.is_some());
    }

    /// Upload `background` (when given) and `hud`, with the screen fade on it, into their frame images.
    fn upload_layers(&self, background: Option<&[u32]>, hud: &[u32], fade: Option<[u8; 4]>) {
        let size = wgpu::Extent3d {
            width: FRAME_W as u32,
            height: FRAME_H as u32,
            depth_or_array_layers: 1,
        };
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * FRAME_W as u32),
            rows_per_image: Some(FRAME_H as u32),
        };
        self.queue.write_texture(
            self.overlay_image.0.as_image_copy(),
            &overlay_rgba(hud, fade),
            layout,
            size,
        );
        if let Some(bg) = background {
            self.queue.write_texture(
                self.background_image.0.as_image_copy(),
                &frame_rgba(bg, None),
                layout,
                size,
            );
        }
    }

    /// Background (the CPU image, or the sky behind a scene), the scene, then the HUD.
    fn encode_frame(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        frame: Option<&SceneFrame>,
        cpu_background: bool,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if cpu_background {
            pass.set_pipeline(&self.background);
            pass.set_bind_group(0, &self.background_image.1, &[]);
            pass.draw(0..3, 0..1);
        } else if frame.is_some() {
            pass.set_pipeline(&self.sky_pipe);
            pass.set_bind_group(0, &self.camera_group, &[]);
            pass.set_bind_group(1, &self.sky.0, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(frame) = frame {
            self.draw_scene(&mut pass, frame);
        }
        pass.set_pipeline(&self.hud);
        pass.set_bind_group(0, &self.overlay_image.1, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The composed frame (sky or `background`, scene, HUD) at frame size, read back.
    pub fn compose_frame(
        &mut self,
        scene: &LdbScene,
        view: Option<GpuView>,
        use_lightmaps: bool,
        characters: &[GpuCharacter],
        background: Option<&[u32]>,
        hud: &[u32],
    ) -> Vec<u32> {
        let frame = view.map(|v| self.prepare_scene(scene, v, use_lightmaps, characters));
        self.upload_layers(background, hud, scene.screen_fade.map(|fade| fade.color()));
        let color = self.color.create_view(&Default::default());
        let depth = self.depth.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.encode_frame(
            &mut encoder,
            &color,
            &depth,
            frame.as_ref(),
            background.is_some(),
        );
        let (w, h) = (FRAME_W as u32, FRAME_H as u32);
        encoder.copy_texture_to_buffer(
            self.color.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.color_out,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * w),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.color_out
            .slice(..)
            .map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        let out = match self.color_out.slice(..).get_mapped_range() {
            Ok(bytes) => bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]))
                .collect(),
            Err(_) => Vec::new(),
        };
        self.color_out.unmap();
        out
    }
}

/// Static room meshes grouped into batches (fan-triangulated, world-space positions).
pub fn build_static_batches(scene: &LdbScene) -> Vec<StaticBatch> {
    let mut batches = Vec::new();
    for sm in &scene.static_meshes {
        add_polys(
            scene,
            &sm.polys,
            sm.world_origin,
            None,
            sm.room_id,
            &mut batches,
        );
    }
    batches
}

/// Texture, lightmap, alpha reference, writes depth, detail offset, dynamic mesh, blended, room.
type BatchKey = (
    Option<usize>,
    Option<usize>,
    Option<u8>,
    bool,
    i32,
    Option<usize>,
    bool,
    Option<i32>,
);

fn add_polys(
    scene: &LdbScene,
    polys: &[crate::scene::ScenePoly],
    o: [f32; 3],
    mesh: Option<usize>,
    room: Option<i32>,
    batches: &mut Vec<StaticBatch>,
) {
    let mut index: BTreeMap<BatchKey, usize> = batches
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let detail = (b.depth_bias / DETAIL_OFFSET_M).round() as i32;
            (
                (
                    b.texture,
                    b.lightmap,
                    b.alpha_reference,
                    b.writes_z,
                    detail,
                    b.mesh,
                    b.blended,
                    b.room,
                ),
                i,
            )
        })
        .collect();
    for poly in polys {
        let policy = poly_policy(scene, poly.material_id);
        if !policy.draw || poly.vertices.len() < 3 {
            continue;
        }
        let material = scene.materials.get(&poly.material_id);
        let texture = material.and_then(|m| m.texture_index);
        let alpha_reference = material.and_then(|m| m.alpha_reference);
        let lightmap = (poly.lightmap_id >= 0).then_some(poly.lightmap_id as usize);
        let blended = alpha_reference.is_some() && material.is_some_and(|m| m.blended);
        let key = (
            texture,
            lightmap,
            alpha_reference,
            policy.writes_z,
            policy.detail_offset,
            mesh,
            blended,
            room,
        );
        let at = *index.entry(key).or_insert_with(|| {
            batches.push(StaticBatch {
                texture,
                lightmap,
                alpha_reference,
                writes_z: policy.writes_z,
                depth_bias: policy.detail_offset as f32 * DETAIL_OFFSET_M,
                mesh,
                room,
                blended,
                verts: Vec::new(),
            });
            batches.len() - 1
        });
        let world = |p: [f32; 3]| [p[0] + o[0], p[1] + o[1], p[2] + o[2]];
        let cull_ref = world(poly.vertices[0].pos);
        let gv = |k: usize| {
            let v = &poly.vertices[k];
            GpuVertex {
                pos: world(v.pos),
                uv: v.uv,
                luv: v.luv,
                cull_ref,
                normal: poly.normal,
                light: [1.0; 3],
            }
        };
        let out = &mut batches[at].verts;
        for k in 1..poly.vertices.len() - 1 {
            out.extend([gv(0), gv(k), gv(k + 1)]);
        }
    }
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
                blended: false,
            },
        );
        let batches = build_static_batches(&scene);
        assert_eq!(batches.len(), 2);
        assert!(batches.iter().any(|b| b.lightmap.is_none()));
        assert!(batches.iter().any(|b| b.lightmap == Some(0)));
    }

    fn yawed_door(hidden: bool) -> crate::scene::SceneDynamicMesh {
        let mut door = crate::render3d::test_glass_quad(hidden);
        door.name = "::startroom::door.DO".into();
        // Yaw 30 degrees, so the shade comes from the rotated normal.
        let (s, c) = (0.5f32, 0.866_025_4f32);
        door.world_mat = [c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c];
        door.local_start_mat = door.world_mat;
        door
    }

    #[test]
    fn dynamic_meshes_get_local_space_batches_tagged_with_their_mesh() {
        let mut scene = crate::render3d::test_empty_scene();
        scene.dynamic_meshes.push(yawed_door(false));
        let batches = build_dynamic_batches(&scene);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].mesh, Some(0));
        // Local space: the transform is applied per frame on the GPU.
        assert_eq!(batches[0].verts[0].pos, [0.0, 0.0, -1.0]);
        assert_eq!(batches[0].verts.len(), 6);
    }

    #[test]
    fn gpu_dynamic_mesh_matches_the_software_frame_and_hides() {
        let mut scene = crate::render3d::test_empty_scene();
        scene.dynamic_meshes.push(yawed_door(false));
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!(
                "no GPU adapter: gpu_dynamic_mesh_matches_the_software_frame_and_hides skipped"
            );
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (sw_fb, sw_zb) = crate::render3d::test_software_dynamic(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        let covered = |z: &[f32], i: usize| z[i].is_finite();
        let sw_cov = (0..w * h).filter(|&i| covered(&sw_zb, i)).count();
        let agree = (0..w * h)
            .filter(|&i| covered(&sw_zb, i) == covered(&zb, i))
            .count();
        assert!(sw_cov > 1000, "the door must cover pixels");
        assert!(
            agree as f32 / (w * h) as f32 > 0.99,
            "coverage agreement {agree}"
        );
        let center = (h / 2) * w + w / 2;
        assert_eq!(fb[center], sw_fb[center], "same rotated-normal shade");

        scene.dynamic_meshes[0].hidden = true;
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        assert!(
            zb.iter().all(|z| z.is_infinite()),
            "a hidden mesh draws nothing"
        );
    }

    #[test]
    fn a_renderer_rebuilt_for_another_scene_keeps_drawing_on_the_same_device() {
        let menu = crate::render3d::test_empty_scene();
        let Some(gpu) = GpuRenderer::new(&menu) else {
            eprintln!("no GPU adapter: renderer rebuild test skipped");
            return;
        };
        let mut level = crate::render3d::test_empty_scene();
        level.dynamic_meshes.push(yawed_door(false));
        let mut gpu = gpu.for_scene(&level);
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&level, view, true, &[], &mut fb, &mut zb);
        assert!(zb.iter().filter(|z| z.is_finite()).count() > 1000);
    }

    fn quad_rig() -> crate::kf2::CharacterRig {
        use crate::kf2::{KfsFace, KfsMesh};
        let face = |indices| KfsFace {
            indices,
            uvs: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]],
            mat_index: 0,
        };
        crate::kf2::CharacterRig {
            mesh: KfsMesh {
                name: "test".into(),
                // A 1 x 1.8 m board facing -z, feet at y = 0.
                vertices: vec![
                    [-0.5, 0.0, 0.0],
                    [0.5, 0.0, 0.0],
                    [0.5, 1.8, 0.0],
                    [-0.5, 1.8, 0.0],
                ],
                // Wound so that (b - a) x (c - a) = -z, the KFS front.
                faces: vec![face([0, 2, 1]), face([0, 3, 2])],
                material_textures: vec![None],
            },
            skin: None,
            idle: None,
            walk: None,
            bind: None,
        }
    }

    #[test]
    fn characters_hide_their_back_faces() {
        // A camera inside Max's head saw the inside of his face. KFS fronts are wound so that
        // (b - a) x (c - a) points out of the mesh (188 of 193 head triangles of Payne_Max_L0).
        let rig = quad_rig();
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let front = crate::render3d::test_view([0.0, 1.0, -3.0], [0.0, 0.0, 1.0]);
        let back = crate::render3d::test_view([0.0, 1.0, 3.0], [0.0, 0.0, -1.0]);
        let covered = |zb: &[f32]| zb.iter().filter(|z| z.is_finite()).count();
        let (_, sw_front) = crate::render3d::test_software_character(&rig, [0.0; 3], false, front);
        let (_, sw_back) = crate::render3d::test_software_character(&rig, [0.0; 3], false, back);
        assert!(covered(&sw_front) > 1000, "software front");
        assert_eq!(covered(&sw_back), 0, "software back");
        let scene = crate::render3d::test_empty_scene();
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: characters_hide_their_back_faces GPU half skipped");
            return;
        };
        for (view, want_drawn) in [(front, true), (back, false)] {
            let verts =
                crate::render3d::character_world_verts(&rig, [0.0; 3], 0.0, 0.0, false, false);
            let light = vec![[1.0; 3]; verts.len()];
            let character = GpuCharacter {
                mesh: &rig.mesh,
                verts,
                dead: false,
                light,
            };
            let mut fb = vec![0u32; w * h];
            let mut zb = vec![f32::INFINITY; w * h];
            gpu.render(&scene, view, true, &[character], &mut fb, &mut zb);
            assert_eq!(covered(&zb) > 1000, want_drawn, "gpu drawn={want_drawn}");
        }
    }

    #[test]
    fn gpu_character_matches_the_software_character() {
        let scene = crate::render3d::test_empty_scene();
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_character_matches_the_software_character skipped");
            return;
        };
        let rig = quad_rig();
        let view = crate::render3d::test_view([0.0, 1.0, -3.0], [0.0, 0.0, 1.0]);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        for hit_flash in [false, true] {
            let (sw_fb, sw_zb) =
                crate::render3d::test_software_character(&rig, [0.0; 3], hit_flash, view);
            let verts =
                crate::render3d::character_world_verts(&rig, [0.0; 3], 0.0, 0.0, false, false);
            let light = crate::render3d::character_vertex_light(
                &rig.mesh,
                &verts,
                [0.0; 3],
                hit_flash,
                &[],
                [0.0; 3],
            );
            let character = GpuCharacter {
                mesh: &rig.mesh,
                verts,
                dead: false,
                light,
            };
            let mut fb = vec![0u32; w * h];
            let mut zb = vec![f32::INFINITY; w * h];
            gpu.render(&scene, view, true, &[character], &mut fb, &mut zb);
            let covered = |z: &[f32], i: usize| z[i].is_finite();
            let sw_cov = (0..w * h).filter(|&i| covered(&sw_zb, i)).count();
            let agree = (0..w * h)
                .filter(|&i| covered(&sw_zb, i) == covered(&zb, i))
                .count();
            assert!(sw_cov > 1000, "the board must cover pixels");
            assert!(
                agree as f32 / (w * h) as f32 > 0.99,
                "coverage agreement {agree}"
            );
            let center = (h / 2) * w + w / 2;
            let channel_gap = (0..3)
                .map(|k| {
                    ((fb[center] >> (8 * k)) & 0xff).abs_diff((sw_fb[center] >> (8 * k)) & 0xff)
                })
                .max()
                .unwrap_or(0);
            assert!(
                channel_gap <= 1,
                "shade {hit_flash}: {:06x} vs {:06x}",
                fb[center],
                sw_fb[center]
            );
        }
    }

    fn gradient_sky() -> crate::scene::DecodedTexture {
        let (width, height) = (64u32, 16u32);
        crate::scene::DecodedTexture {
            path: "sky.tga".into(),
            flag: 0,
            width,
            height,
            pixels: (0..width * height)
                .map(|i| ((i % width) * 4) << 16 | ((i / width) * 16) << 8 | 0x40)
                .collect(),
        }
    }

    #[test]
    fn gpu_sky_matches_the_software_sky() {
        let scene = crate::render3d::test_empty_scene();
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_sky_matches_the_software_sky skipped");
            return;
        };
        let sky = gradient_sky();
        gpu.set_sky(Some(&sky));
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let view = crate::render3d::test_view([0.0, 1.0, 0.0], [0.6, 0.3, 0.74]);
        let cpu = crate::render3d::test_software_sky(&sky, view);
        let hud = vec![HUD_CLEAR; w * h];
        let gpu_frame = gpu.compose_frame(&scene, Some(view), true, &[], None, &hud);
        let same = cpu.iter().zip(&gpu_frame).filter(|(a, b)| a == b).count();
        // Nearest-texel boundaries may land one texel apart; everything else matches.
        assert!(same as f32 / (w * h) as f32 > 0.97, "sky agreement {same}");
    }

    #[test]
    fn without_a_sky_the_frame_has_the_flat_two_tone_background() {
        let scene = crate::render3d::test_empty_scene();
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!(
                "no GPU adapter: without_a_sky_the_frame_has_the_flat_two_tone_background skipped"
            );
            return;
        };
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let view = crate::render3d::test_view([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
        let mut hud = vec![HUD_CLEAR; w * h];
        hud[(h / 2) * w + 7] = 0x00ff_dd66;
        let frame = gpu.compose_frame(&scene, Some(view), true, &[], None, &hud);
        assert_eq!(frame[w * 10 + 10], 0x0016_1c28, "upper half");
        assert_eq!(frame[w * (h - 10) + 10], 0x001e_1e22, "lower half");
        assert_eq!(frame[(h / 2) * w + 7], 0x00ff_dd66, "HUD pixel on top");
    }

    #[test]
    fn screen_fade_goes_into_the_hud_overlay() {
        // `MPHM_FadeToColor` covers scene and HUD, as `render3d::blend_screen` does in software.
        let hud = [HUD_CLEAR, 0x00ff_dd66];
        assert_eq!(overlay_rgba(&hud, None), frame_rgba(&hud, Some(HUD_CLEAR)));
        assert_eq!(
            overlay_rgba(&hud, Some([0, 0, 0, 255])),
            vec![0, 0, 0, 255, 0, 0, 0, 255]
        );
        assert_eq!(
            overlay_rgba(&hud, Some([255, 255, 255, 128])),
            vec![255, 255, 255, 128, 0xff, 0xee, 0xb3, 255]
        );
    }

    #[test]
    fn hud_clear_pixels_become_transparent_and_drawn_ones_opaque() {
        let hud = [HUD_CLEAR, 0x00ff_dd66, 0x0000_0000];
        assert_eq!(
            frame_rgba(&hud, Some(HUD_CLEAR)),
            vec![0, 0, 0, 0, 0xff, 0xdd, 0x66, 0xff, 0, 0, 0, 0xff]
        );
        // The background keeps every pixel opaque.
        assert!(frame_rgba(&hud, None).chunks(4).all(|c| c[3] == 0xff));
    }

    #[test]
    fn gpu_blended_plate_matches_the_software_blend() {
        // A white texel at alpha 128 on a BlendedAlphaTest material, over a black frame.
        let mut scene = crate::render3d::test_empty_scene();
        scene.textures.push(crate::scene::DecodedTexture {
            path: "plate".into(),
            flag: 4,
            width: 1,
            height: 1,
            pixels: vec![0x80ff_ffff],
        });
        scene.materials.insert(
            1,
            SceneMaterial {
                id: 1,
                category: "externalwall".into(),
                texture_path: String::new(),
                texture_index: Some(0),
                alpha_reference: Some(15),
                blended: true,
            },
        );
        let mut plate = crate::render3d::test_glass_quad(false);
        plate.polys[0].material_id = 1;
        scene.dynamic_meshes.push(plate);
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_blended_plate_matches_the_software_blend skipped");
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (sw_fb, _) = crate::render3d::test_software_dynamic(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        let center = (h / 2) * w + w / 2;
        assert!(sw_fb[center] != 0, "the plate must draw");
        let gap = (0..3)
            .map(|k| ((fb[center] >> (8 * k)) & 0xff).abs_diff((sw_fb[center] >> (8 * k)) & 0xff))
            .max()
            .unwrap_or(0);
        assert!(gap <= 1, "{:06x} vs {:06x}", fb[center], sw_fb[center]);
    }

    #[test]
    fn gpu_fog_matches_the_software_fog() {
        let mut scene = scene_with(vec![quad(7, -1)], [0.0; 3]);
        scene.static_meshes[0].polys[0].normal = [-1.0, 0.0, 0.0];
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_fog_matches_the_software_fog skipped");
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (mut sw_fb, sw_zb) = crate::render3d::test_software_static(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let center = (h / 2) * w + w / 2;
        let clear = sw_fb[center];
        // The quad is 4 m away: half way through a 0..8 m fog.
        scene.fog = Some(crate::scene::LevelFog {
            color: [200.0, 40.0, 40.0],
            start: 0.0,
            end: 8.0,
        });
        crate::render3d::apply_fog(&mut sw_fb, &sw_zb, scene.fog);
        assert_ne!(sw_fb[center], clear, "the fog must tint the quad");
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        let gap = (0..3)
            .map(|k| ((fb[center] >> (8 * k)) & 0xff).abs_diff((sw_fb[center] >> (8 * k)) & 0xff))
            .max()
            .unwrap_or(0);
        assert!(gap <= 1, "{:06x} vs {:06x}", fb[center], sw_fb[center]);
    }

    #[test]
    fn gpu_lightmap_filtering_matches_the_software_lightmap() {
        // A black-to-white 2x1 lightmap stretched across the quad: both renderers blend it.
        let mut poly = quad(7, 0);
        for v in &mut poly.vertices {
            v.luv = [(v.pos[2] + 1.0) * 0.5, 0.5];
        }
        let mut scene = scene_with(vec![poly], [0.0; 3]);
        scene.lightmaps.push(crate::scene::DecodedLightmap {
            width: 2,
            height: 1,
            pixels: vec![0x0000_0000, 0x00ff_ffff],
        });
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!(
                "no GPU adapter: gpu_lightmap_filtering_matches_the_software_lightmap skipped"
            );
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (sw_fb, sw_zb) = crate::render3d::test_software_static(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        let row = (h / 2) * w;
        let covered: Vec<usize> = (row..row + w).filter(|&i| sw_zb[i].is_finite()).collect();
        assert!(covered.len() > 20, "the quad must cover the middle row");
        let close = covered
            .iter()
            .filter(|&&i| {
                (0..3).all(|k| {
                    ((fb[i] >> (8 * k)) & 0xff).abs_diff((sw_fb[i] >> (8 * k)) & 0xff) <= 3
                })
            })
            .count();
        assert!(
            close as f32 / covered.len() as f32 > 0.95,
            "{close} of {}",
            covered.len()
        );
    }

    #[test]
    fn mip_chain_halves_each_level_with_a_box_filter() {
        let levels = mip_chain(2, 2, &[0xff00_0000, 0xff00_0000, 0xffff_ffff, 0x00ff_ffff]);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], (1, 1, vec![0xbf80_8080]));
        // Non-square: 4x1 -> 2x1 -> 1x1.
        let levels = mip_chain(4, 1, &[0, 0x0004_0404, 0x0008_0808, 0x000c_0c0c]);
        assert_eq!(
            levels.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(),
            [(4, 1), (2, 1), (1, 1)]
        );
        assert_eq!(levels[2].2, vec![0x0006_0606]);
    }

    #[test]
    fn gpu_texture_filtering_matches_the_software_texture() {
        // A black-to-white 2x1 texture stretched across the quad: both renderers blend it.
        let mut poly = quad(1, -1);
        for v in &mut poly.vertices {
            v.uv = [(v.pos[2] + 1.0) * 0.5, 0.5];
        }
        let mut scene = scene_with(vec![poly], [0.0; 3]);
        scene.textures.push(crate::scene::DecodedTexture {
            path: "ramp".into(),
            flag: 0,
            width: 2,
            height: 1,
            pixels: vec![0xff00_0000, 0xffff_ffff],
        });
        scene.materials.insert(
            1,
            SceneMaterial {
                id: 1,
                category: "wall".into(),
                texture_path: String::new(),
                texture_index: Some(0),
                alpha_reference: None,
                blended: false,
            },
        );
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_texture_filtering_matches_the_software_texture skipped");
            return;
        };
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let (sw_fb, sw_zb) = crate::render3d::test_software_static(&scene, view);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        let row = (h / 2) * w;
        let covered: Vec<usize> = (row..row + w).filter(|&i| sw_zb[i].is_finite()).collect();
        assert!(covered.len() > 20, "the quad must cover the middle row");
        let distinct: std::collections::BTreeSet<u32> =
            covered.iter().map(|&i| sw_fb[i] & 0xff).collect();
        assert!(
            distinct.len() > 20,
            "software must blend, not step: {distinct:?}"
        );
        let close = covered
            .iter()
            .filter(|&&i| {
                (0..3).all(|k| {
                    ((fb[i] >> (8 * k)) & 0xff).abs_diff((sw_fb[i] >> (8 * k)) & 0xff) <= 3
                })
            })
            .count();
        assert!(
            close as f32 / covered.len() as f32 > 0.95,
            "{close} of {}",
            covered.len()
        );
    }

    #[test]
    fn a_level_camera_sees_only_its_room_group() {
        // startup_level: the opening water scene (::start, ::help) overlaps the street
        // (::startroom, ::teleport01) in space. Street cameras showed the water scene's floor.
        let mut scene = scene_with(vec![quad(7, -1)], [0.0; 3]);
        scene.static_meshes[0].room_id = Some(2);
        let room = |id: i32| crate::scene::SceneRoom {
            id,
            name: format!("::room{id}"),
            static_meshes: Vec::new(),
            exits: Vec::new(),
            dynamic_meshes: Vec::new(),
            world_origin: [0.0; 3],
        };
        scene.rooms = vec![room(1), room(2)];
        let mut cam = crate::render3d::test_glass_quad(true);
        cam.room_id = 1;
        scene.dynamic_meshes.push(cam);
        let view = crate::render3d::test_view([-4.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let covered = |zb: &[f32]| zb.iter().filter(|z| z.is_finite()).count();
        // The free player camera does not track its room: every room is drawn.
        let (_, sw_zb) = crate::render3d::test_software_static(&scene, view);
        assert!(covered(&sw_zb) > 1000, "free camera");
        scene.attached_camera = Some(0);
        let (_, sw_zb) = crate::render3d::test_software_static(&scene, view);
        assert_eq!(covered(&sw_zb), 0, "software, camera in room 1");
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: a_level_camera_sees_only_its_room_group GPU half skipped");
            return;
        };
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let mut fb = vec![0u32; w * h];
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        assert_eq!(covered(&zb), 0, "gpu, camera in room 1");
        scene.attached_camera = None;
        let mut zb = vec![f32::INFINITY; w * h];
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
        assert!(covered(&zb) > 1000, "gpu, free camera");
    }

    #[test]
    fn gpu_sprites_match_the_software_sprites() {
        use crate::particles::{Sprite, SpriteTexture};
        let scene = crate::render3d::test_empty_scene();
        let Some(mut gpu) = GpuRenderer::new(&scene) else {
            eprintln!("no GPU adapter: gpu_sprites_match_the_software_sprites skipped");
            return;
        };
        let white = crate::scene::DecodedTexture {
            path: String::new(),
            flag: 0,
            width: 1,
            height: 1,
            pixels: vec![0xffff_ffff],
        };
        let view = crate::render3d::test_view([0.0; 3], [0.0, 0.0, 1.0]);
        let (w, h) = (crate::render3d::FRAME_W, crate::render3d::FRAME_H);
        let centre = (h / 2) * w + w / 2;
        for (rgba, additive) in [([1.0, 1.0, 1.0, 0.5], false), ([1.0, 0.25, 0.0, 0.5], true)] {
            let sprite = Sprite {
                pos: [0.0, 0.0, 5.0],
                size: 2.0,
                rgba,
                additive,
                texture: SpriteTexture {
                    effect: "k".into(),
                    emitter: 0,
                    sequence: 0,
                    frame: 0,
                },
            };
            let mut sw = vec![0u32; w * h];
            let mut sw_zb = vec![f32::INFINITY; w * h];
            crate::render3d::draw_sprites(
                &mut sw,
                &mut sw_zb,
                view,
                h as f32 * 0.85,
                std::slice::from_ref(&sprite),
                |_| Some(&white),
            );
            gpu.set_sprites(std::slice::from_ref(&sprite), view, |_| Some(&white));
            let mut fb = vec![0u32; w * h];
            let mut zb = vec![f32::INFINITY; w * h];
            gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
            assert!(sw[centre] != 0, "the software sprite must draw");
            let gap = (0..3)
                .map(|k| ((fb[centre] >> (8 * k)) & 0xff).abs_diff((sw[centre] >> (8 * k)) & 0xff))
                .max()
                .unwrap_or(0);
            assert!(
                gap <= 2,
                "additive {additive}: {:06x} vs {:06x}",
                fb[centre],
                sw[centre]
            );
        }
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
        gpu.render(&scene, view, true, &[], &mut fb, &mut zb);
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
