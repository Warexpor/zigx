use std::sync::Arc;

use glyphon::{
    Attrs, Buffer, Cache, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache, TextArea,
    TextAtlas, TextBounds, TextRenderer, Viewport, Weight, Wrap,
};
use wgpu::util::DeviceExt;
use wgpu::*;
use winit::window::Window;

use zigx::{DrawList, Label};

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
    @location(4) params: vec4<f32>,
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
    // Grow the quad so soft edges are not clipped to the axis-aligned rect.
    // Fill AA (~0.75px), border band (bw + ~0.9), and optional drop shadow
    // all paint outside the logical shape — without pad, ghost outlines look
    // faceted and 1px rings alias hard.
    let bw = max(ins.params.y, 0.0);
    let shadow = ins.params.z;
    let edge_pad = max(1.5, bw + 2.0);
    let shadow_pad = select(0.0, shadow + 4.0, shadow > 0.5);
    let pad = max(edge_pad, shadow_pad);
    let pos = ins.rect.xy - pad + uv * (size + 2.0 * pad);
    var out: VertexOut;
    out.clip = ndc(pos);
    out.local = uv * (size + 2.0 * pad) - pad;
    out.size = size;
    out.fill = ins.fill;
    out.border = ins.border;
    out.params = ins.params;
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
    let cover = 1.0 - smoothstep(-0.75, 0.75, dist);

    // Flat fill. No sheen, no lift: the compositor blur is the only light.
    let fill_a = v.fill.a * cover;

    // Hairline border straddling the edge, uniform all the way round.
    let band = 1.0 - smoothstep(max(bw - 0.5, 0.0), bw + 0.9, abs(dist));
    let border_a = v.border.a * band * step(0.001, bw);

    // Compose fill then border, premultiplied.
    var col_a = fill_a;
    var col_rgb = v.fill.rgb * fill_a;
    col_rgb = v.border.rgb * border_a + col_rgb * (1.0 - border_a);
    col_a = border_a + col_a * (1.0 - border_a);

    // Soft drop shadow, only outside the box, shifted slightly downward.
    var shadow_a = 0.0;
    if v.params.z > 0.5 {
        let sd = sd_round_box(p - vec2<f32>(0.0, v.params.z * 0.25), size * 0.5, radius);
        shadow_a = v.params.w * (1.0 - smoothstep(0.0, v.params.z, sd)) * (1.0 - cover);
    }
    let a = col_a + shadow_a * (1.0 - col_a);
    return vec4<f32>(col_rgb, a);
}
"#;

const STROKE: &str = r#"
struct Globals {
    resolution: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Vin {
    // xy = pixel position. z = signed distance from the centerline in px.
    // w = solid half-width in px; negative marks a wash vertex.
    @location(0) pos: vec4<f32>,
    @location(1) color: vec4<f32>,
    // Stroke: overshoot past the start (x) and end (y) of the segment, in px.
    // Negative inside. Huge negative means that end has no cap.
    // Wash: (trace y, baseline y) in px.
    @location(2) axis: vec2<f32>,
};

struct Vout {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) edge: vec2<f32>,
    @location(2) axis: vec2<f32>,
};

@vertex
fn vs_main(v: Vin) -> Vout {
    let x = v.pos.x / globals.resolution.x * 2.0 - 1.0;
    let y = 1.0 - v.pos.y / globals.resolution.y * 2.0;
    var out: Vout;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.color = v.color;
    out.edge = v.pos.zw;
    out.axis = v.axis;
    return out;
}

@fragment
fn fs_main(v: Vout) -> @location(0) vec4<f32> {
    if v.edge.y < 0.0 {
        // Wash under a trace. axis = (trace y at this column, baseline y); both
        // are linear across each slice, so the fade is exact per pixel and has
        // no seam along the triangle split, however steep the slice.
        let span = max(v.axis.y - v.axis.x, 1.0);
        let t = clamp((v.axis.y - v.clip.y) / span, 0.0, 1.0);
        // Static screen-space dither: a gradient this faint bands in 8 bits.
        let n = fract(sin(dot(floor(v.clip.xy), vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
        return max(v.color * t + vec4<f32>(n / 255.0), vec4<f32>(0.0));
    }
    // Capsule distance: perpendicular inside the run, radial past a capped end.
    let over = max(max(v.axis.x, v.axis.y), 0.0);
    let dist = length(vec2<f32>(over, abs(v.edge.x)));
    // Soft 1px fringe past the solid half-width. Matches the SDF slab AA.
    let cover = 1.0 - smoothstep(v.edge.y - 0.55, v.edge.y + 0.55, dist);
    return v.color * cover;
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
    axis: [f32; 2],
}

struct TextLayer {
    renderer: TextRenderer,
    buffers: Vec<Buffer>,
    keys: Vec<(String, u32, bool, u16, i32)>,
}

impl TextLayer {
    fn new(atlas: &mut TextAtlas, device: &Device) -> Self {
        Self {
            renderer: TextRenderer::new(atlas, device, MultisampleState::default(), None),
            buffers: Vec::new(),
            keys: Vec::new(),
        }
    }
}

struct Prepared {
    index: usize,
    left: f32,
    top: f32,
    bounds: TextBounds,
    color: glyphon::Color,
}

pub struct Gfx {
    instance: Instance,
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
    /// Per draw layer: end of its shape instances and stroke vertices.
    layer_ends: [(u32, u32); 2],
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    /// One text batch per draw layer, so overlay text sits above base shapes.
    text: [TextLayer; 2],
    fonts: Fonts,
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
        // Non-sRGB target on purpose: the theme's alpha tokens are authored
        // with web semantics (8% white over black is #141414, not #4d4d4d).
        // An sRGB format would re-encode the shader's linear output and lift
        // every ghost fill and hairline several stops. glyphon picks its
        // Web color mode for these formats, so text agrees.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .unwrap_or(TextureFormat::Bgra8Unorm);
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
        let fonts = choose_families(&mut font_system);
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&device);
        let viewport = Viewport::new(&device, &cache);
        let mut atlas = TextAtlas::new(&device, &queue, &cache, format);
        let text = [
            TextLayer::new(&mut atlas, &device),
            TextLayer::new(&mut atlas, &device),
        ];

        Self {
            instance,
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
            layer_ends: [(0, 0); 2],
            font_system,
            swash_cache,
            viewport,
            atlas,
            text,
            fonts,
            logged_text_error: false,
        }
    }

    /// Draw one frame. Returns true when the swapchain was recreated and the
    /// caller should paint again; the frame just attempted was not presented.
    pub fn render(&mut self, window: &Arc<Window>, draw: &DrawList, scale: f32) -> bool {
        let physical = window.inner_size();
        if physical.width == 0 || physical.height == 0 {
            return false;
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
        self.instance_cpu.clear();
        self.vertex_cpu.clear();
        for (i, layer) in draw.layers.iter().enumerate() {
            self.push_shapes(&layer.slabs, scale);
            self.push_strokes(&layer.strokes, scale);
            self.layer_ends[i] = (self.instance_cpu.len() as u32, self.vertex_cpu.len() as u32);
        }
        self.upload_geometry();
        self.viewport.update(
            &self.queue,
            Resolution {
                width: self.config.width,
                height: self.config.height,
            },
        );
        for (i, layer) in draw.layers.iter().enumerate() {
            self.prepare_text(i, &layer.labels, scale);
        }

        let mut configure_after = false;
        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            // The texture is still presentable. Configure afterwards so the
            // next frame matches the surface.
            CurrentSurfaceTexture::Suboptimal(frame) => {
                configure_after = true;
                frame
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => return false,
            CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return true;
            }
            // Lost is not fixed by configure: the surface itself has to be
            // created again or the window stays blank until a resize.
            CurrentSurfaceTexture::Lost => {
                if self.recreate_surface(window) {
                    return true;
                }
                return false;
            }
            CurrentSurfaceTexture::Validation => return false,
        };
        let view = frame.texture.create_view(&TextureViewDescriptor::default());
        let device = &self.device;
        let shape_pipeline = &self.shape_pipeline;
        let stroke_pipeline = &self.stroke_pipeline;
        let bind_group = &self.bind_group;
        let instance_buf = &self.instance_buf;
        let vertex_buf = &self.vertex_buf;
        let layer_ends = self.layer_ends;
        let text = &self.text;
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
            let mut start = (0u32, 0u32);
            for (i, &(inst_end, vert_end)) in layer_ends.iter().enumerate() {
                // Text rendering binds glyphon's atlas at group 0; restore ours
                // before every layer's geometry.
                pass.set_bind_group(0, bind_group, &[]);
                if inst_end > start.0 {
                    pass.set_pipeline(shape_pipeline);
                    pass.set_vertex_buffer(0, instance_buf.slice(..));
                    pass.draw(0..6, start.0..inst_end);
                }
                if vert_end > start.1 {
                    pass.set_pipeline(stroke_pipeline);
                    pass.set_vertex_buffer(0, vertex_buf.slice(..));
                    pass.draw(start.1..vert_end, 0..1);
                }
                if let Err(err) = text[i].renderer.render(atlas, viewport, &mut pass) {
                    text_err = Some(err);
                }
                start = (inst_end, vert_end);
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
        if configure_after {
            self.surface.configure(&self.device, &self.config);
        }
        false
    }

    fn recreate_surface(&mut self, window: &Arc<Window>) -> bool {
        let Ok(surface) = self.instance.create_surface(window.clone()) else {
            return false;
        };
        surface.configure(&self.device, &self.config);
        self.surface = surface;
        true
    }

    fn push_shapes(&mut self, slabs: &[zigx::Slab], scale: f32) {
        for s in slabs {
            self.instance_cpu.push(ShapeInstance {
                rect: [s.x * scale, s.y * scale, s.w * scale, s.h * scale],
                fill: straight(s.fill),
                border: straight(s.border),
                params: [
                    s.radius * scale,
                    s.border_w * scale,
                    s.shadow * scale,
                    s.shadow_a,
                ],
            });
        }
    }

    fn push_strokes(&mut self, strokes: &[zigx::Stroke], scale: f32) {
        for stroke in strokes {
            let pts: Vec<[f32; 2]> = stroke
                .pts
                .iter()
                .map(|p| [p[0] * scale, p[1] * scale])
                .collect();
            let color = premul(stroke.color);
            if let Some(base) = stroke.baseline {
                // Faint wash under the line: tinted at the trace, gone at the baseline.
                let mut top = stroke.color;
                top[3] = ((stroke.color[3] as f32 * 0.07).min(18.0)) as u8;
                fill_under(&pts, base * scale, premul(top), &mut self.vertex_cpu);
            }
            stroke_line(
                &pts,
                (stroke.width * scale).max(1.0),
                stroke.round,
                color,
                &mut self.vertex_cpu,
            );
        }
    }

    fn upload_geometry(&mut self) {
        self.ensure_instances(self.instance_cpu.len());
        if !self.instance_cpu.is_empty() {
            self.queue.write_buffer(
                &self.instance_buf,
                0,
                bytemuck::cast_slice(&self.instance_cpu),
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

    fn prepare_text(&mut self, layer: usize, labels: &[Label], scale: f32) {
        let Gfx {
            font_system,
            swash_cache,
            viewport,
            atlas,
            device,
            queue,
            fonts,
            text,
            ..
        } = &mut *self;
        let tl = &mut text[layer];
        let n = labels.len();
        while tl.buffers.len() < n {
            let mut buf = Buffer::new(font_system, Metrics::new(14.0, 18.0));
            buf.set_wrap(Wrap::None);
            tl.buffers.push(buf);
            tl.keys.push((String::new(), 0, false, 400, 0));
        }
        tl.buffers.truncate(n);
        tl.keys.truncate(n);
        let mut prepared = Vec::with_capacity(n);
        for (i, label) in labels.iter().enumerate() {
            let size_px = (label.size * scale).max(1.0);
            let size_key = (size_px * 10.0).round() as u32;
            let track_key = (label.tracking * 1000.0).round() as i32;
            let changed = tl.keys[i].0 != label.text
                || tl.keys[i].1 != size_key
                || tl.keys[i].2 != label.mono
                || tl.keys[i].3 != label.weight
                || tl.keys[i].4 != track_key;
            if changed {
                let weight = if label.mono {
                    fonts.mono.snap(label.weight)
                } else {
                    fonts.sans.snap(label.weight)
                };
                let mut attrs = if label.mono {
                    Attrs::new().family(Family::Monospace)
                } else {
                    Attrs::new().family(Family::SansSerif)
                }
                .weight(Weight(weight));
                if label.tracking != 0.0 {
                    attrs = attrs.letter_spacing(label.tracking);
                }
                let buf = &mut tl.buffers[i];
                buf.set_metrics(Metrics::new(size_px, (label.h * scale).max(size_px)));
                buf.set_size(
                    Some((label.w * scale).max(1.0)),
                    Some((label.h * scale).max(1.0)),
                );
                buf.set_text(&label.text, &attrs, Shaping::Advanced, None);
                buf.shape_until_scroll(font_system, false);
                tl.keys[i] = (
                    label.text.clone(),
                    size_key,
                    label.mono,
                    label.weight,
                    track_key,
                );
            }
            let x = (label.x * scale).round();
            let y = (label.y * scale).round();
            let mut bounds = TextBounds {
                left: (x as i32) - 1,
                top: (y as i32) - 1,
                right: (x + label.w * scale).ceil() as i32 + 2,
                bottom: (y + label.h * scale).ceil() as i32 + 4,
            };
            if let Some(clip) = label.clip {
                let left = (clip.x * scale).floor() as i32;
                let top = (clip.y * scale).floor() as i32;
                let right = (clip.right() * scale).ceil() as i32;
                let bottom = (clip.bottom() * scale).ceil() as i32;
                bounds.left = bounds.left.max(left);
                bounds.top = bounds.top.max(top);
                bounds.right = bounds.right.min(right);
                bounds.bottom = bounds.bottom.min(bottom);
                if bounds.right <= bounds.left || bounds.bottom <= bounds.top {
                    continue;
                }
            }
            prepared.push(Prepared {
                index: i,
                left: x,
                top: y,
                bounds,
                color: glyphon::Color::rgba(
                    label.color[0],
                    label.color[1],
                    label.color[2],
                    label.color[3],
                ),
            });
        }
        let empty: &[glyphon::CustomGlyph] = &[];
        let areas: Vec<TextArea> = prepared
            .iter()
            .filter_map(|p| {
                Some(TextArea {
                    buffer: tl.buffers.get(p.index)?,
                    left: p.left,
                    top: p.top,
                    scale: 1.0,
                    bounds: p.bounds,
                    default_color: p.color,
                    custom_glyphs: empty,
                })
            })
            .collect();
        if let Err(err) = tl.renderer.prepare(
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
}

/// Upright weights a family can actually produce. `None` means the family
/// has a variable `wght` axis and any weight is fine.
#[derive(Clone, Debug, Default)]
struct WeightSet(Option<Vec<u16>>);

impl WeightSet {
    /// Nearest available weight. cosmic-text only treats exact (or variable)
    /// matches as belonging to the requested family; anything else drops
    /// into a machine-wide fallback list where an italic face can win a tie.
    fn snap(&self, want: u16) -> u16 {
        match &self.0 {
            None => want,
            Some(list) if list.is_empty() => want,
            Some(list) => *list
                .iter()
                .min_by_key(|w| (w.abs_diff(want), **w))
                .unwrap_or(&want),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Fonts {
    sans: WeightSet,
    mono: WeightSet,
}

/// cosmic-text's built-in generic families ("Open Sans", "Noto Sans Mono") are
/// rarely installed. When the family is missing, every face on the machine is
/// ranked by weight distance alone, so a 600-weight label can land in a serif.
/// Bind the generic families to fonts that actually exist instead.
fn choose_families(font_system: &mut FontSystem) -> Fonts {
    const SANS: &[&str] = &[
        "Inter",
        "Inter Variable",
        "Adwaita Sans",
        "Cantarell",
        "Noto Sans",
        "Liberation Sans",
        "DejaVu Sans",
    ];
    const MONO: &[&str] = &[
        "JetBrainsMono Nerd Font",
        "JetBrains Mono",
        "Adwaita Mono",
        "CaskaydiaMono Nerd Font",
        "Cascadia Mono",
        "Fira Code",
        "Noto Sans Mono",
        "Liberation Mono",
        "DejaVu Sans Mono",
    ];
    let installed: std::collections::HashSet<String> = font_system
        .db()
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
        .collect();
    let pick = |env: &str, prefs: &[&str]| -> Option<String> {
        if let Ok(name) = std::env::var(env) {
            if installed.contains(&name) {
                return Some(name);
            }
            eprintln!("zigx: {env}={name} is not an installed font family");
        }
        prefs
            .iter()
            .find(|name| installed.contains(**name))
            .map(|name| name.to_string())
    };
    let sans = pick("ZIGX_SANS", SANS);
    let mono = pick("ZIGX_MONO", MONO);
    let fonts = Fonts {
        sans: sans
            .as_deref()
            .map(|n| weights_of(font_system, n))
            .unwrap_or_default(),
        mono: mono
            .as_deref()
            .map(|n| weights_of(font_system, n))
            .unwrap_or_default(),
    };
    let db = font_system.db_mut();
    if let Some(name) = &sans {
        db.set_sans_serif_family(name.clone());
    }
    if let Some(name) = &mono {
        db.set_monospace_family(name.clone());
    }
    fonts
}

fn weights_of(font_system: &FontSystem, family: &str) -> WeightSet {
    let db = font_system.db();
    let mut weights: Vec<u16> = Vec::new();
    for face in db.faces() {
        if face.style != glyphon::Style::Normal
            || !face.families.iter().any(|(name, _)| name == family)
        {
            continue;
        }
        let variable = db
            .with_face_data(face.id, |data, index| {
                ttf_parser::Face::parse(data, index)
                    .map(|f| {
                        f.variation_axes()
                            .into_iter()
                            .any(|a| a.tag == ttf_parser::Tag::from_bytes(b"wght"))
                    })
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if variable {
            return WeightSet(None);
        }
        weights.push(face.weight.0);
    }
    weights.sort_unstable();
    weights.dedup();
    WeightSet(Some(weights))
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
        attributes: &vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x2],
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

/// Wash from the trace down to `baseline`, faded per pixel in the shader.
fn fill_under(pts: &[[f32; 2]], baseline: f32, color: [f32; 4], out: &mut Vec<Vert>) {
    let wash = |out: &mut Vec<Vert>, p: [f32; 2], trace_y: f32| {
        out.push(Vert {
            pos: [p[0], p[1], 0.0, -1.0],
            color,
            axis: [trace_y, baseline],
        });
    };
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        wash(out, a, a[1]);
        wash(out, b, b[1]);
        wash(out, [a[0], baseline], a[1]);
        wash(out, b, b[1]);
        wash(out, [b[0], baseline], b[1]);
        wash(out, [a[0], baseline], a[1]);
    }
}

/// Sentinel for "this end has no cap": the fragment never sees it go positive.
const NO_CAP: f32 = -1.0e6;

/// One ribbon vertex. `dist` is the signed perpendicular distance from the
/// centerline; `sa`/`sb` are overshoots past the segment's start and end.
fn stroke_vert(
    out: &mut Vec<Vert>,
    p: [f32; 2],
    dist: f32,
    half_w: f32,
    sa: f32,
    sb: f32,
    color: [f32; 4],
) {
    out.push(Vert {
        pos: [p[0], p[1], dist, half_w],
        color,
        axis: [sa, sb],
    });
}

/// Polyline as one joined ribbon. Consecutive segments meet at miter points
/// instead of overlapping, so semi-transparent ink stays even through the
/// corners, and `round` grows capsule ends in the fragment shader rather
/// than stacking a second primitive on top.
fn stroke_line(pts: &[[f32; 2]], width: f32, round: bool, color: [f32; 4], out: &mut Vec<Vert>) {
    let hw = (width * 0.5).max(0.5);
    // Grow the ribbon so the fragment soft-edge has pixels to fade across.
    let aa = 0.75_f32;
    let outer = hw + aa;

    // Collapse repeated points so every segment has a direction.
    let mut p: Vec<[f32; 2]> = Vec::with_capacity(pts.len());
    for q in pts {
        let dup = p
            .last()
            .is_some_and(|l: &[f32; 2]| (l[0] - q[0]).abs() < 1e-3 && (l[1] - q[1]).abs() < 1e-3);
        if !dup {
            p.push(*q);
        }
    }
    let n = p.len();
    if n < 2 {
        return;
    }
    let dirs: Vec<[f32; 2]> = p
        .windows(2)
        .map(|w| {
            let dx = w[1][0] - w[0][0];
            let dy = w[1][1] - w[0][1];
            let l = (dx * dx + dy * dy).sqrt();
            [dx / l, dy / l]
        })
        .collect();
    let normal = |d: [f32; 2]| [-d[1], d[0]];
    // A path that ends where it starts is closed: the seam is one more miter
    // join, with no caps stacking ink over it.
    let closed =
        n >= 4 && (p[0][0] - p[n - 1][0]).abs() < 1e-3 && (p[0][1] - p[n - 1][1]).abs() < 1e-3;
    let round = round && !closed;

    // Left-side offset at every point: plain normal at open ends, miter inside.
    let offs: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let (d0, d1) = if i == 0 || i == n - 1 {
                if !closed {
                    let nn = normal(dirs[if i == 0 { 0 } else { n - 2 }]);
                    return [nn[0] * outer, nn[1] * outer];
                }
                (dirs[n - 2], dirs[0])
            } else {
                (dirs[i - 1], dirs[i])
            };
            let n0 = normal(d0);
            let n1 = normal(d1);
            let mx = n0[0] + n1[0];
            let my = n0[1] + n1[1];
            let ml = (mx * mx + my * my).sqrt();
            if ml < 1e-3 {
                // Full reversal: no meaningful miter, fall back to the normal.
                return [n0[0] * outer, n0[1] * outer];
            }
            let m = [mx / ml, my / ml];
            // Half-angle cosine. Acute turns use a bevel (no /cos stretch) so
            // graph spikes do not throw a long miter that flickers under AA.
            let cos = (m[0] * n0[0] + m[1] * n0[1]).clamp(0.0, 1.0);
            let len = if cos < 0.72 { outer } else { outer / cos };
            [m[0] * len, m[1] * len]
        })
        .collect();

    for i in 0..n - 1 {
        let a = p[i];
        let b = p[i + 1];
        let d = dirs[i];
        let len = (b[0] - a[0]) * d[0] + (b[1] - a[1]) * d[1];
        let cap_a = round && i == 0;
        let cap_b = round && i == n - 2;
        let ext_a = if cap_a { outer } else { 0.0 };
        let ext_b = if cap_b { outer } else { 0.0 };
        let corners = [
            (
                [
                    a[0] + offs[i][0] - d[0] * ext_a,
                    a[1] + offs[i][1] - d[1] * ext_a,
                ],
                outer,
            ),
            (
                [
                    a[0] - offs[i][0] - d[0] * ext_a,
                    a[1] - offs[i][1] - d[1] * ext_a,
                ],
                -outer,
            ),
            (
                [
                    b[0] + offs[i + 1][0] + d[0] * ext_b,
                    b[1] + offs[i + 1][1] + d[1] * ext_b,
                ],
                outer,
            ),
            (
                [
                    b[0] - offs[i + 1][0] + d[0] * ext_b,
                    b[1] - offs[i + 1][1] + d[1] * ext_b,
                ],
                -outer,
            ),
        ];
        // Along-axis projection of each corner gives exact overshoot varyings.
        let along = |q: [f32; 2]| (q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1];
        let emit = |out: &mut Vec<Vert>, k: usize| {
            let (q, dist) = corners[k];
            let t = along(q);
            let sa = if cap_a { -t } else { NO_CAP };
            let sb = if cap_b { t - len } else { NO_CAP };
            stroke_vert(out, q, dist, hw, sa, sb, color);
        };
        // Two triangles: (a_l, b_l, a_r) and (b_l, b_r, a_r).
        emit(out, 0);
        emit(out, 2);
        emit(out, 1);
        emit(out, 2);
        emit(out, 3);
        emit(out, 1);
    }
}
