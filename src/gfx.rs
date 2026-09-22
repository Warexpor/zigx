use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use wgpu::util::DeviceExt;
use wgpu::*;
use winit::window::Window;

use zigx::DrawList;

const SHAPE: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) fill: vec4<f32>,
    @location(2) border: vec4<f32>,
    @location(3) params: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec2<f32>,
};

fn ndc(p: vec2<f32>) -> vec4<f32> {
    let x = p.x / globals.resolution.x * 2.0 - 1.0;
    let y = 1.0 - p.y / globals.resolution.y * 2.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, ins: Instance) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let uv = corners[vi];
    let size = ins.rect.zw;
    let pos = ins.rect.xy + uv * size;
    var out: VertexOut;
    out.clip = ndc(pos);
    out.local = uv * size;
    out.size = size;
    out.fill = ins.fill;
    out.border = ins.border;
    out.params = ins.params.xy;
    return out;
}

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
}

@fragment
fn fs_main(v: VertexOut) -> @location(0) vec4<f32> {
    let size = max(v.size, vec2<f32>(1.0));
    let radius = min(v.params.x, min(size.x, size.y) * 0.5);
    let bw = max(v.params.y, 0.0);
    let p = v.local - size * 0.5;
    let dist = sd_round_box(p, size * 0.5, radius);
    let cover = 1.0 - smoothstep(-0.8, 0.6, dist);
    let band = 1.0 - smoothstep(bw, bw + 1.2, abs(dist));
    let top = 1.0 - smoothstep(0.0, size.y * 0.72, v.local.y);
    let fill_a = v.fill.a * cover;
    let border_a = v.border.a * band * (0.4 + 0.6 * top);
    let fill_rgb = v.fill.rgb + vec3<f32>(top * 0.04);
    let border_rgb = mix(v.border.rgb * 0.45, min(v.border.rgb + vec3<f32>(0.25), vec3<f32>(1.0)), top);
    var premul = vec4<f32>(fill_rgb * fill_a, fill_a);
    let brim = vec4<f32>(border_rgb * border_a, border_a);
    premul = brim + premul * (1.0 - brim.a);
    return premul;
}
"#;

const STROKE: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Vin {
    @location(0) pos: vec4<f32>,
    @location(1) color: vec4<f32>,
};

struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(v: Vin) -> Vout {
    let x = v.pos.x / globals.resolution.x * 2.0 - 1.0;
    let y = 1.0 - v.pos.y / globals.resolution.y * 2.0;
    var out: Vout;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.color = v.color;
    return out;
}

@fragment
fn fs_main(v: Vout) -> @location(0) vec4<f32> {
    return v.color;
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    resolution: [f32; 2],
    pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ShapeInstance {
    rect: [f32; 4],
    fill: [f32; 4],
    border: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vert {
    pos: [f32; 4],
    color: [f32; 4],
}

struct Prepared {
    index: usize,
    left: f32,
    top: f32,
    bounds: TextBounds,
    color: glyphon::Color,
}

pub struct Gfx {
    surface: Surface<'static>,
    device: Device,
    queue: Queue,
    config: SurfaceConfiguration,
    shape_pipeline: RenderPipeline,
    stroke_pipeline: RenderPipeline,
    bind_group: BindGroup,
    globals: wgpu::Buffer,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    vertex_buf: wgpu::Buffer,
    vertex_cap: usize,
    instance_cpu: Vec<ShapeInstance>,
    vertex_cpu: Vec<Vert>,
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    text_renderer: TextRenderer,
    buffers: Vec<Buffer>,
    buffer_keys: Vec<(String, u32, bool)>,
    prepared: Vec<Prepared>,
    logged_text_error: bool,
}

impl Gfx {
    pub fn new(window: Arc<Window>, event_loop: &winit::event_loop::ActiveEventLoop) -> Self {
        let instance = wgpu::Instance::new(InstanceDescriptor::new_with_display_handle(Box::new(
            event_loop.owned_display_handle(),
        )));
        let surface = instance
            .create_surface(window.clone())
            .expect("window surface");
        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .expect("gpu adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&DeviceDescriptor::default()))
                .expect("gpu device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(TextureFormat::Bgra8UnormSrgb);
        let alpha = if caps
            .alpha_modes
            .contains(&CompositeAlphaMode::PreMultiplied)
        {
            CompositeAlphaMode::PreMultiplied
        } else if caps
            .alpha_modes
            .contains(&CompositeAlphaMode::PostMultiplied)
        {
            CompositeAlphaMode::PostMultiplied
        } else if caps.alpha_modes.contains(&CompositeAlphaMode::Inherit) {
            CompositeAlphaMode::Inherit
        } else {
            CompositeAlphaMode::Auto
        };
        eprintln!("zigx surface {format:?} alpha {alpha:?}");

        let size = window.inner_size();
        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: PresentMode::Fifo,
            alpha_mode: alpha,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);

        let globals = device.create_buffer_init(&util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                resolution: [config.width as f32, config.height as f32],
                pad: [0.0, 0.0],
            }),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
        let bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("globals"),
            layout: &bgl,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("zigx"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let shape_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("shape"),
            source: ShaderSource::Wgsl(SHAPE.into()),
        });
        let stroke_shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("stroke"),
            source: ShaderSource::Wgsl(STROKE.into()),
        });
        let premul = BlendState::PREMULTIPLIED_ALPHA_BLENDING;
        let shape_pipeline = pipeline(&device, &layout, &shape_shader, format, premul, true);
        let stroke_pipeline = pipeline(&device, &layout, &stroke_shader, format, premul, false);

        let instance_cap = 256;
        let vertex_cap = 4096;
        let instance_buf = device.create_buffer(&BufferDescriptor {
            label: Some("instances"),
            size: (instance_cap * std::mem::size_of::<ShapeInstance>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let vertex_buf = device.create_buffer(&BufferDescriptor {
            label: Some("strokes"),
            size: (vertex_cap * std::mem::size_of::<Vert>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text_renderer =
            TextRenderer::new(&mut atlas, &device, MultisampleState::default(), None);

        // Keep the cache alive; glyphon stores what it needs inside the atlas.
        drop(cache);
        let _ = &mut font_system;

        Self {
            surface,
            device,
            queue,
            config,
            shape_pipeline,
            stroke_pipeline,
            bind_group,
            globals,
            instance_buf,
            instance_cap,
            vertex_buf,
            vertex_cap,
            instance_cpu: Vec::new(),
            vertex_cpu: Vec::new(),
            font_system,
            swash_cache,
            viewport,
            atlas,
            text_renderer,
            buffers: Vec::new(),
            buffer_keys: Vec::new(),
            prepared: Vec::new(),
            logged_text_error: false,
        }
    }

    pub fn render(&mut self, window: &Window, draw: &DrawList) {
        let physical = window.inner_size();
        let scale = window.scale_factor() as f32;
        if physical.width == 0 || physical.height == 0 {
            return;
        }
        if self.config.width != physical.width || self.config.height != physical.height {
            self.config.width = physical.width.max(1);
            self.config.height = physical.height.max(1);
            self.surface.configure(&self.device, &self.config);
        }
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&Globals {
                resolution: [self.config.width as f32, self.config.height as f32],
                pad: [0.0, 0.0],
            }),
        );
        self.upload_shapes(draw, scale);
        self.upload_strokes(draw, scale);
        self.prepare_text(draw, scale);

        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            CurrentSurfaceTexture::Lost => return,
            CurrentSurfaceTexture::Validation => return,
        };
        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let device = &self.device;
        let shape_pipeline = &self.shape_pipeline;
        let stroke_pipeline = &self.stroke_pipeline;
        let bind_group = &self.bind_group;
        let instance_buf = &self.instance_buf;
        let vertex_buf = &self.vertex_buf;
        let n_inst = self.instance_cpu.len() as u32;
        let n_vert = self.vertex_cpu.len() as u32;
        let text_renderer = &self.text_renderer;
        let atlas = &self.atlas;
        let viewport = &self.viewport;
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("zigx"),
        });
        let mut text_err = None;
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("zigx"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, bind_group, &[]);
            if n_inst > 0 {
                pass.set_pipeline(shape_pipeline);
                pass.set_vertex_buffer(0, instance_buf.slice(..));
                pass.draw(0..6, 0..n_inst);
            }
            if n_vert > 0 {
                pass.set_pipeline(stroke_pipeline);
                pass.set_vertex_buffer(0, vertex_buf.slice(..));
                pass.draw(0..n_vert, 0..1);
            }
            if let Err(err) = text_renderer.render(atlas, viewport, &mut pass) {
                text_err = Some(err);
            }
        }
        if let Some(err) = text_err {
            if !self.logged_text_error {
                eprintln!("zigx text: {err}");
                self.logged_text_error = true;
            }
        }
        self.queue.submit(Some(encoder.finish()));
        window.pre_present_notify();
        self.queue.present(frame);
        self.atlas.trim();
    }

    fn upload_shapes(&mut self, draw: &DrawList, scale: f32) {
        self.instance_cpu.clear();
        for s in &draw.slabs {
            self.instance_cpu.push(ShapeInstance {
                rect: [s.x * scale, s.y * scale, s.w * scale, s.h * scale],
                fill: straight(s.fill),
                border: straight(s.border),
                params: [s.radius * scale, s.border_w * scale, 0.0, 0.0],
            });
        }
        self.ensure_instances(self.instance_cpu.len());
        if !self.instance_cpu.is_empty() {
            self.queue.write_buffer(
                &self.instance_buf,
                0,
                bytemuck::cast_slice(&self.instance_cpu),
            );
        }
    }

    fn upload_strokes(&mut self, draw: &DrawList, scale: f32) {
        self.vertex_cpu.clear();
        for stroke in &draw.strokes {
            let pts: Vec<[f32; 2]> = stroke
                .pts
                .iter()
                .map(|p| [p[0] * scale, p[1] * scale])
                .collect();
            let color = premul(stroke.color);
            if let Some(base) = stroke.baseline {
                let mut fill = stroke.color;
                fill[3] = 36;
                fill_under(&pts, base * scale, premul(fill), &mut self.vertex_cpu);
            }
            stroke_line(
                &pts,
                (stroke.width * scale).max(1.0),
                color,
                &mut self.vertex_cpu,
            );
        }
        self.ensure_verts(self.vertex_cpu.len());
        if !self.vertex_cpu.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buf, 0, bytemuck::cast_slice(&self.vertex_cpu));
        }
    }

    fn ensure_instances(&mut self, n: usize) {
        if n <= self.instance_cap {
            return;
        }
        self.instance_cap = n.next_power_of_two().max(64);
        self.instance_buf = self.device.create_buffer(&BufferDescriptor {
            label: Some("instances"),
            size: (self.instance_cap * std::mem::size_of::<ShapeInstance>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn ensure_verts(&mut self, n: usize) {
        if n <= self.vertex_cap {
            return;
        }
        self.vertex_cap = n.next_power_of_two().max(1024);
        self.vertex_buf = self.device.create_buffer(&BufferDescriptor {
            label: Some("strokes"),
            size: (self.vertex_cap * std::mem::size_of::<Vert>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
    }

    fn prepare_text(&mut self, draw: &DrawList, scale: f32) {
        let n = draw.labels.len();
        while self.buffers.len() < n {
            let mut buf = Buffer::new(&mut self.font_system, Metrics::new(14.0, 18.0));
            buf.set_wrap(Wrap::None);
            self.buffers.push(buf);
            self.buffer_keys.push((String::new(), 0, false));
        }
        self.buffers.truncate(n);
        self.buffer_keys.truncate(n);
        self.prepared.clear();
        let mut dirty = vec![false; n];
        for (i, label) in draw.labels.iter().enumerate() {
            let size_px = (label.size * scale).max(1.0);
            let size_key = (size_px * 10.0).round() as u32;
            let changed = self.buffer_keys[i].0 != label.text
                || self.buffer_keys[i].1 != size_key
                || self.buffer_keys[i].2 != label.mono;
            if changed {
                let attrs = if label.mono {
                    Attrs::new().family(Family::Monospace)
                } else {
                    Attrs::new().family(Family::SansSerif)
                };
                let buf = &mut self.buffers[i];
                buf.set_metrics(Metrics::new(size_px, (label.h * scale).max(size_px)));
                buf.set_size(
                    Some((label.w * scale).max(1.0)),
                    Some((label.h * scale).max(1.0)),
                );
                buf.set_text(&label.text, &attrs, Shaping::Advanced, None);
                self.buffer_keys[i] = (label.text.clone(), size_key, label.mono);
                dirty[i] = true;
            }
            let x = (label.x * scale).round();
            let y = (label.y * scale).round();
            self.prepared.push(Prepared {
                index: i,
                left: x,
                top: y,
                bounds: TextBounds {
                    left: (x as i32) - 1,
                    top: (y as i32) - 1,
                    right: (x + label.w * scale).ceil() as i32 + 2,
                    bottom: (y + label.h * scale).ceil() as i32 + 4,
                },
                color: glyphon::Color::rgba(
                    label.color[0],
                    label.color[1],
                    label.color[2],
                    label.color[3],
                ),
            });
        }
        {
            let Gfx {
                buffers,
                font_system,
                ..
            } = &mut *self;
            for (i, buf) in buffers.iter_mut().enumerate() {
                if dirty.get(i).copied().unwrap_or(false) {
                    buf.shape_until_scroll(font_system, false);
                }
            }
        }
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        let prepared = std::mem::take(&mut self.prepared);
        {
            let Gfx {
                buffers,
                text_renderer,
                font_system,
                atlas,
                viewport,
                device,
                queue,
                swash_cache,
                ..
            } = &mut *self;
            let empty: &[glyphon::CustomGlyph] = &[];
            let areas: Vec<TextArea> = prepared
                .iter()
                .filter_map(|p| {
                    Some(TextArea {
                        buffer: buffers.get(p.index)?,
                        left: p.left,
                        top: p.top,
                        scale: 1.0,
                        bounds: p.bounds,
                        default_color: p.color,
                        custom_glyphs: empty,
                    })
                })
                .collect();
            if let Err(err) = text_renderer.prepare(
                device,
                queue,
                font_system,
                atlas,
                viewport,
                areas,
                swash_cache,
            ) {
                eprintln!("zigx text prepare: {err}");
            }
        }
        self.prepared = prepared;
    }
}

fn pipeline(
    device: &Device,
    layout: &PipelineLayout,
    shader: &ShaderModule,
    format: TextureFormat,
    blend: BlendState,
    instanced: bool,
) -> RenderPipeline {
    let shape_layout = [Some(VertexBufferLayout {
        array_stride: std::mem::size_of::<ShapeInstance>() as u64,
        step_mode: VertexStepMode::Instance,
        attributes: &vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4],
    })];
    let stroke_layout = [Some(VertexBufferLayout {
        array_stride: std::mem::size_of::<Vert>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: &vertex_attr_array![0 => Float32x4, 1 => Float32x4],
    })];
    let buffers: &[Option<VertexBufferLayout>] = if instanced {
        &shape_layout
    } else {
        &stroke_layout
    };
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(if instanced { "shape" } else { "stroke" }),
        layout: Some(layout),
        vertex: VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers,
        },
        primitive: PrimitiveState {
            topology: PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: MultisampleState::default(),
        fragment: Some(FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn straight(c: [u8; 4]) -> [f32; 4] {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        c[3] as f32 / 255.0,
    ]
}

fn premul(c: [u8; 4]) -> [f32; 4] {
    let a = c[3] as f32 / 255.0;
    [
        c[0] as f32 / 255.0 * a,
        c[1] as f32 / 255.0 * a,
        c[2] as f32 / 255.0 * a,
        a,
    ]
}

fn fill_under(pts: &[[f32; 2]], baseline: f32, color: [f32; 4], out: &mut Vec<Vert>) {
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        tri(out, [a[0], a[1]], [b[0], b[1]], [a[0], baseline], color);
        tri(out, [b[0], b[1]], [b[0], baseline], [a[0], baseline], color);
    }
}

fn stroke_line(pts: &[[f32; 2]], width: f32, color: [f32; 4], out: &mut Vec<Vert>) {
    let hw = width * 0.5;
    for w in pts.windows(2) {
        let (x0, y0) = (w[0][0], w[0][1]);
        let (x1, y1) = (w[1][0], w[1][1]);
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(0.001);
        let nx = -dy / len * hw;
        let ny = dx / len * hw;
        tri(
            out,
            [x0 + nx, y0 + ny],
            [x1 + nx, y1 + ny],
            [x0 - nx, y0 - ny],
            color,
        );
        tri(
            out,
            [x1 + nx, y1 + ny],
            [x1 - nx, y1 - ny],
            [x0 - nx, y0 - ny],
            color,
        );
    }
}

fn tri(out: &mut Vec<Vert>, a: [f32; 2], b: [f32; 2], c: [f32; 2], color: [f32; 4]) {
    for p in [a, b, c] {
        out.push(Vert {
            pos: [p[0], p[1], 0.0, 1.0],
            color,
        });
    }
}
