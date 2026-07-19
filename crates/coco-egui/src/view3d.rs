//! The 3D desk view (`docs/plan-3d-world.md`, task 1 — steps 1–2 of the
//! incremental path): the running VM presented as a physical desk — CRT
//! monitor, CoCo 3 case, a cartridge — rendered with raw OpenGL inside the
//! normal egui `CentralPanel` via a `glow` paint callback, with an orbit
//! camera. The live `coco-fb` texture (the same one `draw_display` shows
//! flat) is mapped onto the CRT face, so the 3D view is a pure *view*: no
//! emulation state is touched and the flat view stays byte-identical.
//!
//! Props load from `assets3d/<slug>.glb` (git-ignored, like `roms/`) when
//! present; a missing or unreadable file degrades to a colored placeholder
//! box of roughly period-correct dimensions, so the whole code path works
//! with zero assets on disk.
//!
//! GL resources are created lazily inside the first paint callback (the only
//! place a `glow::Context` is guaranteed to exist — `CocoApp::new` runs
//! without a `CreationContext`, see its doc comment) and freed in
//! `CocoApp::on_exit`. A VM window closed by the manager mid-session leaks
//! its handful of buffers until process exit — acceptable while this view is
//! experimental. egui_glow re-runs `prepare_painting` after every callback,
//! so depth test / blend / program state set here needs no manual restore.

use std::path::Path;
use std::sync::{Arc, Mutex};

use eframe::egui;
use eframe::glow::{self, HasContext as _};
use glam::{Mat3, Mat4, Vec3};

/// Depth bits requested from eframe (`NativeOptions::depth_buffer`) — eframe
/// defaults to 0 and egui itself never depth-tests, but the desk scene does.
pub const DEPTH_BUFFER_BITS: u8 = 24;

// ---------------------------------------------------------------------------
// Desk layout. All dimensions in meters, placeholder-box sizes roughly
// matching the real hardware; replaced visually (not positionally) once a
// modeled `.glb` exists for the prop.
// ---------------------------------------------------------------------------

/// Desk top slab: width, thickness, depth. Top surface sits at y = 0.
const DESK_SIZE: [f32; 3] = [1.60, 0.04, 0.90];
/// CoCo 3 case shell: width, height, depth.
const CASE_SIZE: [f32; 3] = [0.38, 0.08, 0.27];
/// CRT monitor shell (CM-8-ish footprint): width, height, depth.
const MONITOR_SIZE: [f32; 3] = [0.36, 0.34, 0.38];
/// A ROM pak lying on the desk: width, height, depth.
const CART_SIZE: [f32; 3] = [0.09, 0.025, 0.13];
/// Visible CRT glass width; height is fixed 4:3 like the real tube, so the
/// framebuffer stretches to the monitor's aspect exactly as `aspect_correct`
/// does in the flat view.
const SCREEN_WIDTH: f32 = 0.26;
const SCREEN_HEIGHT: f32 = SCREEN_WIDTH * 3.0 / 4.0;
/// Lift of the screen quad off the monitor's front face, avoiding z-fighting.
const SCREEN_LIFT: f32 = 0.002;

/// Monitor center: behind the case, sitting on the desk.
const MONITOR_POS: [f32; 3] = [0.0, MONITOR_SIZE[1] / 2.0, -0.15];
/// Screen center: on the monitor's front face, slightly above monitor center
/// (real CRTs put the tube high, electronics below).
const SCREEN_POS: [f32; 3] = [
    0.0,
    MONITOR_POS[1] + 0.03,
    MONITOR_POS[2] + MONITOR_SIZE[2] / 2.0 + SCREEN_LIFT,
];
/// Case center: in front of the monitor, on the desk.
const CASE_POS: [f32; 3] = [0.0, CASE_SIZE[1] / 2.0, 0.16];
/// Cartridge center: on the desk to the right of the case.
const CART_POS: [f32; 3] = [0.42, CART_SIZE[1] / 2.0, 0.10];
/// Cartridge yaw (radians) so it doesn't sit unnaturally axis-aligned.
const CART_YAW: f32 = -0.4;

const DESK_COLOR: [f32; 4] = [0.45, 0.32, 0.22, 1.0];
const CASE_COLOR: [f32; 4] = [0.82, 0.80, 0.75, 1.0];
const MONITOR_COLOR: [f32; 4] = [0.75, 0.73, 0.68, 1.0];
const CART_COLOR: [f32; 4] = [0.25, 0.25, 0.27, 1.0];
/// Room background the viewport clears to.
const CLEAR_COLOR: [f32; 4] = [0.10, 0.11, 0.13, 1.0];

// ---------------------------------------------------------------------------
// Orbit camera.
// ---------------------------------------------------------------------------

/// Point the camera orbits: roughly the top of the case / bottom of the
/// screen, so both stay framed.
const ORBIT_TARGET: [f32; 3] = [0.0, 0.25, 0.0];
const ORBIT_RADIANS_PER_POINT: f32 = 0.01;
/// Exponential zoom factor per scroll point (trackpad-friendly).
const ZOOM_PER_SCROLL_POINT: f32 = 0.002;
const PITCH_MIN: f32 = -0.05;
const PITCH_MAX: f32 = 1.35;
const DIST_MIN: f32 = 0.4;
const DIST_MAX: f32 = 4.0;
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_4;
const Z_NEAR: f32 = 0.05;
const Z_FAR: f32 = 20.0;

// ---------------------------------------------------------------------------
// CRT shader stack (plan task 2). Six composable passes, each independently
// toggleable from the CRT settings window; strength 0 / toggle off disables a
// pass in the shader (uniform reaches it as 0).
// ---------------------------------------------------------------------------

/// Offscreen phosphor surface the framebuffer is accumulated onto (ping-pong
/// pair for the persistence blend). 4:3 like the tube; finer than any CoCo
/// mode so no detail is lost before the CRT-face pass resamples it.
const PHOSPHOR_SIZE: [i32; 2] = [640, 480];
/// Small blurred copy of the phosphor surface driving bloom + bezel glow.
const GLOW_SIZE: [i32; 2] = [160, 120];
/// The additive glow quad extends this far beyond the screen quad, spilling
/// tube light onto the bezel. Must match `GLOW_QUAD_SCALE` in the fragment
/// shader.
const GLOW_QUAD_SCALE: f32 = 1.5;
/// Lift of the glow quad off the screen quad (which itself sits
/// [`SCREEN_LIFT`] off the monitor face).
const GLOW_QUAD_LIFT: f32 = 0.004;
/// Phosphor decay 1.0 would never fade — cap the settings slider below it.
const PERSISTENCE_MAX: f32 = 0.95;

/// Per-pass toggles and strengths for the CRT look, edited live in the
/// "CRT" settings window. All strengths are 0..=1 except persistence
/// (0..=[`PERSISTENCE_MAX`], it's a decay factor).
#[derive(Clone, Copy)]
pub struct CrtParams {
    pub barrel_on: bool,
    pub barrel: f32,
    pub scanlines_on: bool,
    pub scanlines: f32,
    pub mask_on: bool,
    pub mask: f32,
    pub persistence_on: bool,
    pub persistence: f32,
    pub bloom_on: bool,
    pub bloom: f32,
    pub reflection_on: bool,
    pub reflection: f32,
}

impl Default for CrtParams {
    fn default() -> Self {
        Self {
            barrel_on: true,
            barrel: 0.35,
            scanlines_on: true,
            scanlines: 0.35,
            mask_on: true,
            mask: 0.30,
            persistence_on: true,
            persistence: 0.55,
            bloom_on: true,
            bloom: 0.35,
            reflection_on: true,
            reflection: 0.25,
        }
    }
}

/// The uniform values one frame's callback needs, resolved from
/// [`CrtParams`] on the UI thread (toggle off → 0.0 → pass disabled).
#[derive(Clone, Copy)]
struct CrtUniforms {
    /// barrel, scanlines, mask, bloom — the shader's `u_crt_a`.
    a: [f32; 4],
    /// reflection, source scanline count, unused, unused — `u_crt_b`.
    b: [f32; 4],
    /// Phosphor decay factor for the persistence pass.
    decay: f32,
}

impl CrtParams {
    fn uniforms(&self, fb_lines: f32) -> CrtUniforms {
        let on = |enabled: bool, strength: f32| if enabled { strength } else { 0.0 };
        CrtUniforms {
            a: [
                on(self.barrel_on, self.barrel),
                on(self.scanlines_on, self.scanlines),
                on(self.mask_on, self.mask),
                on(self.bloom_on, self.bloom),
            ],
            b: [on(self.reflection_on, self.reflection), fb_lines, 0.0, 0.0],
            decay: on(self.persistence_on, self.persistence),
        }
    }
}

struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.35,
            pitch: 0.30,
            dist: 1.5,
        }
    }
}

impl OrbitCamera {
    fn view_proj(&self, aspect: f32) -> Mat4 {
        let target = Vec3::from(ORBIT_TARGET);
        let eye = target
            + self.dist
                * Vec3::new(
                    self.pitch.cos() * self.yaw.sin(),
                    self.pitch.sin(),
                    self.pitch.cos() * self.yaw.cos(),
                );
        let view = Mat4::look_at_rh(eye, target, Vec3::Y);
        let proj = Mat4::perspective_rh_gl(FOV_Y_RADIANS, aspect, Z_NEAR, Z_FAR);
        proj * view
    }
}

// ---------------------------------------------------------------------------
// CPU-side meshes and props.
// ---------------------------------------------------------------------------

/// Interleaved vertex data: position (3), normal (3), uv (2).
const FLOATS_PER_VERTEX: usize = 8;
const VERTEX_STRIDE_BYTES: i32 = (FLOATS_PER_VERTEX * std::mem::size_of::<f32>()) as i32;

#[derive(Default)]
struct MeshData {
    verts: Vec<f32>,
    indices: Vec<u32>,
}

/// Axis-aligned cuboid centered on the origin with per-face normals (flat
/// shading) — the universal placeholder.
fn cuboid(size: [f32; 3]) -> MeshData {
    let [hw, hh, hd] = [size[0] / 2.0, size[1] / 2.0, size[2] / 2.0];
    // (normal, four corners CCW seen from outside)
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-hw, -hh, hd], [hw, -hh, hd], [hw, hh, hd], [-hw, hh, hd]]),
        ([0.0, 0.0, -1.0], [[hw, -hh, -hd], [-hw, -hh, -hd], [-hw, hh, -hd], [hw, hh, -hd]]),
        ([1.0, 0.0, 0.0], [[hw, -hh, hd], [hw, -hh, -hd], [hw, hh, -hd], [hw, hh, hd]]),
        ([-1.0, 0.0, 0.0], [[-hw, -hh, -hd], [-hw, -hh, hd], [-hw, hh, hd], [-hw, hh, -hd]]),
        ([0.0, 1.0, 0.0], [[-hw, hh, hd], [hw, hh, hd], [hw, hh, -hd], [-hw, hh, -hd]]),
        ([0.0, -1.0, 0.0], [[-hw, -hh, -hd], [hw, -hh, -hd], [hw, -hh, hd], [-hw, -hh, hd]]),
    ];
    let mut mesh = MeshData::default();
    for (normal, corners) in faces {
        let base = (mesh.verts.len() / FLOATS_PER_VERTEX) as u32;
        for corner in corners {
            mesh.verts.extend_from_slice(&corner);
            mesh.verts.extend_from_slice(&normal);
            mesh.verts.extend_from_slice(&[0.0, 0.0]);
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

/// The CRT face: a +Z-facing quad carrying the framebuffer texture. UV v = 0
/// at the top edge, matching egui's texture orientation (row 0 = top).
fn screen_quad() -> MeshData {
    let (hw, hh) = (SCREEN_WIDTH / 2.0, SCREEN_HEIGHT / 2.0);
    let normal = [0.0, 0.0, 1.0];
    // (pos, uv), CCW from bottom-left.
    let corners: [([f32; 3], [f32; 2]); 4] = [
        ([-hw, -hh, 0.0], [0.0, 1.0]),
        ([hw, -hh, 0.0], [1.0, 1.0]),
        ([hw, hh, 0.0], [1.0, 0.0]),
        ([-hw, hh, 0.0], [0.0, 0.0]),
    ];
    let mut mesh = MeshData::default();
    for (pos, uv) in corners {
        mesh.verts.extend_from_slice(&pos);
        mesh.verts.extend_from_slice(&normal);
        mesh.verts.extend_from_slice(&uv);
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    mesh
}

struct PropDef {
    mesh: MeshData,
    transform: Mat4,
    color: [f32; 4],
    /// The CRT face: textured with the live framebuffer and drawn unlit
    /// (emissive), like a powered tube.
    is_screen: bool,
}

/// Directory the modeled props load from, resolved relative to the process
/// working directory like `roms/` in the boot tests. Git-ignored.
const ASSETS3D_DIR: &str = "assets3d";

/// Load a modeled prop, flattening the glTF node tree (transforms applied,
/// all primitives merged). Returns `None` — placeholder box — on any
/// missing/unreadable/empty file. Materials are ignored for now: modeled
/// props render in the same flat color as their placeholder until the CRT
/// shader / material work (plan tasks 2 and 6).
fn load_gltf_mesh(path: &Path) -> Option<MeshData> {
    let (doc, buffers, _images) = gltf::import(path).ok()?;
    let scene = doc.default_scene().or_else(|| doc.scenes().next())?;
    let mut mesh = MeshData::default();
    for node in scene.nodes() {
        append_node(&mut mesh, &node, Mat4::IDENTITY, &buffers);
    }
    (!mesh.indices.is_empty()).then_some(mesh)
}

fn append_node(
    out: &mut MeshData,
    node: &gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
) {
    let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    let normal_matrix = Mat3::from_mat4(transform);
    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            let reader = prim.reader(|b| buffers.get(b.index()).map(|data| &*data.0));
            let Some(positions) = reader.read_positions() else {
                continue;
            };
            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|it| it.collect())
                .unwrap_or_default();
            let base = (out.verts.len() / FLOATS_PER_VERTEX) as u32;
            let mut vertex_count = 0u32;
            for (i, pos) in positions.enumerate() {
                let p = transform.transform_point3(Vec3::from(pos));
                let n = normals
                    .get(i)
                    .map(|n| (normal_matrix * Vec3::from(*n)).normalize_or_zero())
                    .unwrap_or(Vec3::Y);
                out.verts
                    .extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, 0.0, 0.0]);
                vertex_count += 1;
            }
            match reader.read_indices() {
                Some(indices) => out.indices.extend(indices.into_u32().map(|i| base + i)),
                None => out.indices.extend(base..base + vertex_count),
            }
        }
    }
    for child in node.children() {
        append_node(out, &child, transform, buffers);
    }
}

/// The whole prop set, in draw order. Each solid prop tries
/// `assets3d/<slug>.glb` first; the screen quad is always generated (it
/// carries the framebuffer and must keep its UV mapping).
fn build_props() -> Vec<PropDef> {
    let assets = Path::new(ASSETS3D_DIR);
    let solid = |slug: &str, size: [f32; 3], pos: [f32; 3], yaw: f32, color: [f32; 4]| PropDef {
        mesh: load_gltf_mesh(&assets.join(format!("{slug}.glb")))
            .unwrap_or_else(|| cuboid(size)),
        transform: Mat4::from_translation(Vec3::from(pos)) * Mat4::from_rotation_y(yaw),
        color,
        is_screen: false,
    };
    vec![
        solid(
            "desk",
            DESK_SIZE,
            [0.0, -DESK_SIZE[1] / 2.0, 0.0],
            0.0,
            DESK_COLOR,
        ),
        solid("monitor-cm8", MONITOR_SIZE, MONITOR_POS, 0.0, MONITOR_COLOR),
        solid("coco3-case", CASE_SIZE, CASE_POS, 0.0, CASE_COLOR),
        solid("cartridge", CART_SIZE, CART_POS, CART_YAW, CART_COLOR),
        PropDef {
            mesh: screen_quad(),
            transform: Mat4::from_translation(Vec3::from(SCREEN_POS)),
            color: [0.0, 0.0, 0.0, 1.0], // fallback if the fb texture is gone
            is_screen: true,
        },
    ]
}

/// Where the additive bezel-glow quad sits: over the screen quad, scaled up
/// to spill onto the monitor shell.
fn glow_quad_transform() -> Mat4 {
    Mat4::from_translation(Vec3::from(SCREEN_POS) + Vec3::new(0.0, 0.0, GLOW_QUAD_LIFT))
        * Mat4::from_scale(Vec3::new(GLOW_QUAD_SCALE, GLOW_QUAD_SCALE, 1.0))
}

// ---------------------------------------------------------------------------
// GL resources (live on the glow context, created inside the paint callback).
// ---------------------------------------------------------------------------

const VERTEX_SHADER: &str = r#"#version 150
uniform mat4 u_view_proj;
uniform mat4 u_model;
in vec3 a_pos;
in vec3 a_normal;
in vec2 a_uv;
out vec3 v_normal;
out vec2 v_uv;
void main() {
    v_normal = mat3(u_model) * a_normal;
    v_uv = a_uv;
    gl_Position = u_view_proj * u_model * vec4(a_pos, 1.0);
}
"#;

/// `u_mode` values shared between [`FRAGMENT_SHADER`] and the draw loop.
const MODE_SOLID: i32 = 0;
const MODE_CRT: i32 = 1;
const MODE_GLOW: i32 = 2;

const FRAGMENT_SHADER: &str = r#"#version 150
uniform vec4 u_color;
uniform sampler2D u_tex;   // solid: unused; CRT: phosphor; glow quad: glow
uniform sampler2D u_glow;  // CRT face only: blurred glow for the bloom add
uniform int u_mode;        // 0 solid lit, 1 CRT face, 2 additive glow quad
uniform vec4 u_crt_a;      // barrel, scanlines, mask, bloom strengths
uniform vec4 u_crt_b;      // reflection strength, source scanline count, -, -
in vec3 v_normal;
in vec2 v_uv;
out vec4 frag_color;
const vec3 LIGHT_DIR = vec3(0.35, 0.86, 0.37); // pre-normalized
const float AMBIENT = 0.35;
const float PI = 3.14159265;
// uv displacement toward the corners at barrel strength 1.0
const float BARREL_MAX = 0.5;
// aperture-grille RGB triads across the tube width
const float MASK_TRIADS = 320.0;
// must match Rust's GLOW_QUAD_SCALE
const float GLOW_QUAD_SCALE = 1.5;

// The tube face: barrel-distort the sample position (image bulges, raster
// pulls in from the quad corners), then scanlines and grille on the tube
// pixel, then bloom and the room-light streak on the glass over everything.
vec3 crt_face(vec2 uv) {
    float barrel = u_crt_a.x, scan = u_crt_a.y, mask = u_crt_a.z, bloom = u_crt_a.w;
    float refl = u_crt_b.x, lines = u_crt_b.y;
    vec2 centered = uv - 0.5;
    vec2 tube_uv = 0.5 + centered * (1.0 + barrel * BARREL_MAX * dot(centered, centered) * 2.0);
    vec3 col = vec3(0.0);
    if (all(greaterThanEqual(tube_uv, vec2(0.0))) && all(lessThanEqual(tube_uv, vec2(1.0)))) {
        col = texture(u_tex, tube_uv).rgb;
        col *= 1.0 - scan * 0.5 * (1.0 - cos(tube_uv.y * lines * 2.0 * PI));
        int triad = int(mod(floor(tube_uv.x * MASK_TRIADS * 3.0), 3.0));
        vec3 tint = triad == 0 ? vec3(1.0, 0.6, 0.6)
                  : triad == 1 ? vec3(0.6, 1.0, 0.6)
                               : vec3(0.6, 0.6, 1.0);
        col *= mix(vec3(1.0), tint, mask);
    }
    col += texture(u_glow, uv).rgb * bloom * 0.7;
    float d = dot(uv - vec2(0.30, 0.25), normalize(vec2(0.8, 1.0)));
    col += refl * 0.25 * exp(-d * d * 40.0);
    return col;
}

// The oversized additive quad in front of the bezel: the blurred tube image
// with a radial falloff, so bright screens light the plastic around them.
vec3 glow_quad() {
    vec2 tube = (v_uv - 0.5) * GLOW_QUAD_SCALE + 0.5;
    vec3 g = texture(u_tex, clamp(tube, 0.0, 1.0)).rgb;
    float falloff = smoothstep(1.0, 0.45, length(v_uv - 0.5) * 2.0);
    return g * falloff;
}

void main() {
    if (u_mode == 1) {
        frag_color = vec4(crt_face(v_uv), 1.0);
    } else if (u_mode == 2) {
        frag_color = vec4(glow_quad() * u_crt_a.w * 0.8, 1.0);
    } else {
        float diffuse = max(dot(normalize(v_normal), LIGHT_DIR), 0.0);
        float light = mix(AMBIENT, 1.0, diffuse);
        frag_color = vec4(u_color.rgb * light, u_color.a);
    }
}
"#;

/// Fullscreen-triangle vertex shader for the offscreen passes (no vertex
/// buffer; positions derived from `gl_VertexID`, drawn with an empty VAO).
const BLIT_VERTEX_SHADER: &str = r#"#version 150
out vec2 v_uv;
void main() {
    vec2 pos = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    v_uv = pos * 0.5 + 0.5;
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

/// Pass 0: phosphor persistence — new frame combined with the decayed
/// previous phosphor surface (`max`, like phosphor that re-excites).
/// Pass 1: 9-tap blur of the phosphor surface into the small glow target.
const BLIT_FRAGMENT_SHADER: &str = r#"#version 150
uniform sampler2D u_src;
uniform sampler2D u_prev;
uniform int u_pass;
uniform float u_decay;
uniform vec2 u_texel;
in vec2 v_uv;
out vec4 frag_color;
void main() {
    if (u_pass == 0) {
        vec3 cur = texture(u_src, v_uv).rgb;
        vec3 prev = texture(u_prev, v_uv).rgb * u_decay;
        frag_color = vec4(max(cur, prev), 1.0);
    } else {
        vec2 o = u_texel * 1.6;
        vec3 sum = texture(u_src, v_uv).rgb * 0.2;
        sum += (texture(u_src, v_uv + vec2(o.x, 0.0)).rgb
              + texture(u_src, v_uv - vec2(o.x, 0.0)).rgb
              + texture(u_src, v_uv + vec2(0.0, o.y)).rgb
              + texture(u_src, v_uv - vec2(0.0, o.y)).rgb) * 0.125;
        sum += (texture(u_src, v_uv + o).rgb
              + texture(u_src, v_uv - o).rgb
              + texture(u_src, v_uv + vec2(o.x, -o.y)).rgb
              + texture(u_src, v_uv + vec2(-o.x, o.y)).rgb) * 0.075;
        frag_color = vec4(sum, 1.0);
    }
}
"#;

struct GlMesh {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    index_count: i32,
}

/// A texture + framebuffer pair used as an offscreen render target.
struct RenderTarget {
    tex: glow::Texture,
    fbo: glow::Framebuffer,
}

struct GlScene {
    program: glow::Program,
    u_view_proj: Option<glow::UniformLocation>,
    u_model: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    u_mode: Option<glow::UniformLocation>,
    u_crt_a: Option<glow::UniformLocation>,
    u_crt_b: Option<glow::UniformLocation>,
    /// One entry per prop, same order as the `PropDef` list.
    meshes: Vec<GlMesh>,
    /// Offscreen-pass program (fullscreen triangle, no vertex buffer).
    blit_program: glow::Program,
    u_pass: Option<glow::UniformLocation>,
    u_decay: Option<glow::UniformLocation>,
    /// Core profile requires *a* VAO bound even for buffer-less draws.
    empty_vao: glow::VertexArray,
    /// Phosphor ping-pong pair; `phosphor_prev` indexes last frame's surface.
    phosphor: [RenderTarget; 2],
    phosphor_prev: usize,
    glow_target: RenderTarget,
    /// The additive bezel-glow quad (same geometry as the screen quad).
    glow_mesh: GlMesh,
}

enum GlState {
    Uninit,
    /// Shader compile/link failed — logged once, viewport stays cleared.
    Failed,
    Ready(GlScene),
}

impl GlScene {
    unsafe fn compile_shader(
        gl: &glow::Context,
        kind: u32,
        src: &str,
    ) -> Result<glow::Shader, String> {
        unsafe {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                return Err(log);
            }
            Ok(shader)
        }
    }

    unsafe fn link_program(gl: &glow::Context, vs_src: &str, fs_src: &str) -> Result<glow::Program, String> {
        unsafe {
            let vs = Self::compile_shader(gl, glow::VERTEX_SHADER, vs_src)?;
            let fs = Self::compile_shader(gl, glow::FRAGMENT_SHADER, fs_src)?;
            let program = gl.create_program()?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            gl.detach_shader(program, vs);
            gl.detach_shader(program, fs);
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                return Err(log);
            }
            Ok(program)
        }
    }

    unsafe fn upload_mesh(
        gl: &glow::Context,
        program: glow::Program,
        mesh: &MeshData,
    ) -> Result<GlMesh, String> {
        unsafe {
            let vao = gl.create_vertex_array()?;
            let vbo = gl.create_buffer()?;
            let ebo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_cast(&mesh.verts), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytemuck_cast(&mesh.indices),
                glow::STATIC_DRAW,
            );
            let float_size = std::mem::size_of::<f32>() as i32;
            for (name, components, offset_floats) in
                [("a_pos", 3, 0), ("a_normal", 3, 3), ("a_uv", 2, 6)]
            {
                if let Some(loc) = gl.get_attrib_location(program, name) {
                    gl.enable_vertex_attrib_array(loc);
                    gl.vertex_attrib_pointer_f32(
                        loc,
                        components,
                        glow::FLOAT,
                        false,
                        VERTEX_STRIDE_BYTES,
                        offset_floats * float_size,
                    );
                }
            }
            gl.bind_vertex_array(None);
            Ok(GlMesh {
                vao,
                vbo,
                ebo,
                index_count: mesh.indices.len() as i32,
            })
        }
    }

    /// A linear-filtered, edge-clamped RGBA8 offscreen target.
    unsafe fn create_target(gl: &glow::Context, size: [i32; 2]) -> Result<RenderTarget, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                size[0],
                size[1],
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value);
            }
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("offscreen framebuffer incomplete: 0x{status:x}"));
            }
            Ok(RenderTarget { tex, fbo })
        }
    }

    fn new(gl: &glow::Context, props: &[PropDef]) -> Result<Self, String> {
        unsafe {
            let program = Self::link_program(gl, VERTEX_SHADER, FRAGMENT_SHADER)?;
            let blit_program = Self::link_program(gl, BLIT_VERTEX_SHADER, BLIT_FRAGMENT_SHADER)?;

            let mut meshes = Vec::with_capacity(props.len());
            for prop in props {
                meshes.push(Self::upload_mesh(gl, program, &prop.mesh)?);
            }
            let glow_mesh = Self::upload_mesh(gl, program, &screen_quad())?;

            let empty_vao = gl.create_vertex_array()?;
            let phosphor = [
                Self::create_target(gl, PHOSPHOR_SIZE)?,
                Self::create_target(gl, PHOSPHOR_SIZE)?,
            ];
            let glow_target = Self::create_target(gl, GLOW_SIZE)?;

            // Fixed sampler units: scene u_tex=0 / u_glow=1, blit u_src=0 /
            // u_prev=1. The blur's texel size never changes either.
            gl.use_program(Some(program));
            for (name, unit) in [("u_tex", 0), ("u_glow", 1)] {
                if let Some(loc) = gl.get_uniform_location(program, name) {
                    gl.uniform_1_i32(Some(&loc), unit);
                }
            }
            gl.use_program(Some(blit_program));
            for (name, unit) in [("u_src", 0), ("u_prev", 1)] {
                if let Some(loc) = gl.get_uniform_location(blit_program, name) {
                    gl.uniform_1_i32(Some(&loc), unit);
                }
            }
            gl.uniform_2_f32(
                gl.get_uniform_location(blit_program, "u_texel").as_ref(),
                1.0 / GLOW_SIZE[0] as f32,
                1.0 / GLOW_SIZE[1] as f32,
            );

            Ok(Self {
                u_view_proj: gl.get_uniform_location(program, "u_view_proj"),
                u_model: gl.get_uniform_location(program, "u_model"),
                u_color: gl.get_uniform_location(program, "u_color"),
                u_mode: gl.get_uniform_location(program, "u_mode"),
                u_crt_a: gl.get_uniform_location(program, "u_crt_a"),
                u_crt_b: gl.get_uniform_location(program, "u_crt_b"),
                program,
                meshes,
                u_pass: gl.get_uniform_location(blit_program, "u_pass"),
                u_decay: gl.get_uniform_location(blit_program, "u_decay"),
                blit_program,
                empty_vao,
                phosphor,
                phosphor_prev: 0,
                glow_target,
                glow_mesh,
            })
        }
    }

    /// The two offscreen passes: framebuffer + decayed previous phosphor →
    /// current phosphor surface, then phosphor → small blurred glow target.
    /// Returns the phosphor texture the scene pass should display. Caller
    /// restores framebuffer/viewport/scissor.
    unsafe fn offscreen_passes(
        &mut self,
        gl: &glow::Context,
        fb_texture: glow::Texture,
        decay: f32,
    ) -> glow::Texture {
        unsafe {
            let (prev, cur) = (self.phosphor_prev, 1 - self.phosphor_prev);
            gl.use_program(Some(self.blit_program));
            gl.bind_vertex_array(Some(self.empty_vao));

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.phosphor[cur].fbo));
            gl.viewport(0, 0, PHOSPHOR_SIZE[0], PHOSPHOR_SIZE[1]);
            gl.uniform_1_i32(self.u_pass.as_ref(), 0);
            gl.uniform_1_f32(self.u_decay.as_ref(), decay);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.phosphor[prev].tex));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(fb_texture));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.glow_target.fbo));
            gl.viewport(0, 0, GLOW_SIZE[0], GLOW_SIZE[1]);
            gl.uniform_1_i32(self.u_pass.as_ref(), 1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.phosphor[cur].tex));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            self.phosphor_prev = cur;
            self.phosphor[cur].tex
        }
    }

    /// Draw the frame: offscreen CRT passes, then the prop list, then the
    /// additive bezel glow. Runs inside the paint callback: viewport is
    /// already the panel rect and scissor already clips to it, so the final
    /// clear only touches our pixels; egui_glow restores its own state
    /// afterwards (the offscreen passes restore theirs via `restore_fbo` +
    /// `viewport_px` before the scene draws).
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &mut self,
        gl: &glow::Context,
        props: &[PropDef],
        view_proj: &[f32; 16],
        fb_texture: Option<glow::Texture>,
        crt: CrtUniforms,
        restore_fbo: Option<glow::Framebuffer>,
        viewport_px: [i32; 4],
    ) {
        unsafe {
            // Offscreen passes first (scissor off — they own their whole
            // targets), then restore the on-screen state the callback got.
            let display_tex = fb_texture.map(|fb| {
                gl.disable(glow::SCISSOR_TEST);
                let tex = self.offscreen_passes(gl, fb, crt.decay);
                gl.bind_framebuffer(glow::FRAMEBUFFER, restore_fbo);
                gl.viewport(viewport_px[0], viewport_px[1], viewport_px[2], viewport_px[3]);
                gl.enable(glow::SCISSOR_TEST);
                tex
            });

            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.clear_color(
                CLEAR_COLOR[0],
                CLEAR_COLOR[1],
                CLEAR_COLOR[2],
                CLEAR_COLOR[3],
            );
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.uniform_matrix_4_f32_slice(self.u_view_proj.as_ref(), false, view_proj);
            gl.uniform_4_f32_slice(self.u_crt_a.as_ref(), &crt.a);
            gl.uniform_4_f32_slice(self.u_crt_b.as_ref(), &crt.b);
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.glow_target.tex));
            gl.active_texture(glow::TEXTURE0);

            for (prop, mesh) in props.iter().zip(&self.meshes) {
                let mode = if prop.is_screen && display_tex.is_some() {
                    gl.bind_texture(glow::TEXTURE_2D, display_tex);
                    MODE_CRT
                } else {
                    MODE_SOLID
                };
                gl.uniform_matrix_4_f32_slice(
                    self.u_model.as_ref(),
                    false,
                    &prop.transform.to_cols_array(),
                );
                gl.uniform_4_f32_slice(self.u_color.as_ref(), &prop.color);
                gl.uniform_1_i32(self.u_mode.as_ref(), mode);
                gl.bind_vertex_array(Some(mesh.vao));
                gl.draw_elements(glow::TRIANGLES, mesh.index_count, glow::UNSIGNED_INT, 0);
            }

            // Bezel glow: additive, no depth write (it's light, not a
            // surface), only when there's a powered tube and bloom is on.
            let bloom_strength = crt.a[3];
            if display_tex.is_some() && bloom_strength > 0.0 {
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE);
                gl.depth_mask(false);
                gl.bind_texture(glow::TEXTURE_2D, Some(self.glow_target.tex));
                gl.uniform_matrix_4_f32_slice(
                    self.u_model.as_ref(),
                    false,
                    &glow_quad_transform().to_cols_array(),
                );
                gl.uniform_1_i32(self.u_mode.as_ref(), MODE_GLOW);
                gl.bind_vertex_array(Some(self.glow_mesh.vao));
                gl.draw_elements(
                    glow::TRIANGLES,
                    self.glow_mesh.index_count,
                    glow::UNSIGNED_INT,
                    0,
                );
                gl.depth_mask(true);
                gl.disable(glow::BLEND);
            }
            gl.bind_vertex_array(None);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_program(self.blit_program);
            gl.delete_vertex_array(self.empty_vao);
            for mesh in self.meshes.iter().chain([&self.glow_mesh]) {
                gl.delete_vertex_array(mesh.vao);
                gl.delete_buffer(mesh.vbo);
                gl.delete_buffer(mesh.ebo);
            }
            for target in self.phosphor.iter().chain([&self.glow_target]) {
                gl.delete_framebuffer(target.fbo);
                gl.delete_texture(target.tex);
            }
        }
    }
}

/// Read the just-painted viewport back from the default framebuffer and
/// write it as a PNG (the [`SNAPSHOT_ENV`] one-shot). GL's origin is
/// bottom-left, PNG's top-left, so rows are flipped on the way out.
fn save_snapshot(gl: &glow::Context, info: &egui::PaintCallbackInfo, path: &Path) {
    let vp = info.viewport_in_pixels();
    let (w, h) = (vp.width_px as usize, vp.height_px as usize);
    const RGBA_BYTES: usize = 4;
    let mut pixels = vec![0u8; w * h * RGBA_BYTES];
    unsafe {
        gl.read_pixels(
            vp.left_px,
            vp.from_bottom_px,
            vp.width_px,
            vp.height_px,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
    }
    let row_bytes = w * RGBA_BYTES;
    let mut flipped = Vec::with_capacity(pixels.len());
    for row in pixels.chunks_exact(row_bytes).rev() {
        flipped.extend_from_slice(row);
    }
    match image::save_buffer(
        path,
        &flipped,
        w as u32,
        h as u32,
        image::ColorType::Rgba8,
    ) {
        Ok(()) => tracing::info!("3D desk snapshot written to {}", path.display()),
        Err(e) => tracing::error!("3D desk snapshot failed: {e}"),
    }
}

/// View bytes of a `f32`/`u32` slice for `buffer_data_u8_slice` (safe: both
/// are plain-old-data with no padding).
fn bytemuck_cast<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice))
    }
}

// ---------------------------------------------------------------------------
// The egui-facing view.
// ---------------------------------------------------------------------------

/// Env var naming a PNG path: after [`SNAPSHOT_DELAY_FRAMES`] painted 3D
/// frames (long enough for BASIC to reach its prompt), the frame is read back
/// with `glReadPixels` and written there, then the hook disarms. Debug/verify
/// machinery in the spirit of the headless-typing tricks — it proves what the
/// GL scene actually renders without needing host screen-capture permissions.
const SNAPSHOT_ENV: &str = "COCO_VIEW3D_SNAPSHOT";
const SNAPSHOT_DELAY_FRAMES: u32 = 240;

pub struct View3d {
    /// View → "3D Desk" toggle; off = the classic flat display.
    pub enabled: bool,
    /// View → "CRT Settings…" window visibility.
    pub show_settings: bool,
    /// Live-editable CRT pass toggles/strengths (the settings window).
    pub crt: CrtParams,
    camera: OrbitCamera,
    /// CPU-side props, loaded (glTF or placeholder) on first use.
    props: Option<Arc<Vec<PropDef>>>,
    /// GPU-side scene, created inside the first paint callback and shared
    /// with the per-frame callback closures.
    gl_state: Arc<Mutex<GlState>>,
    /// One-shot [`SNAPSHOT_ENV`] readback target with its frame countdown;
    /// consumed by the callback that paints frame zero of the countdown.
    snapshot: Arc<Mutex<Option<(std::path::PathBuf, u32)>>>,
}

impl View3d {
    pub fn new() -> Self {
        Self {
            enabled: false,
            show_settings: false,
            crt: CrtParams::default(),
            camera: OrbitCamera::default(),
            props: None,
            gl_state: Arc::new(Mutex::new(GlState::Uninit)),
            snapshot: Arc::new(Mutex::new(
                std::env::var_os(SNAPSHOT_ENV)
                    .map(|p| (std::path::PathBuf::from(p), SNAPSHOT_DELAY_FRAMES)),
            )),
        }
    }

    /// Fill the available panel rect with the desk scene, `fb_tex` being the
    /// per-frame `coco-fb` texture (uploaded by `step_emulation`) and
    /// `fb_lines` its height in emulated scanlines (drives the scanline
    /// pass). Dragging orbits, scrolling zooms. Returns the rect it occupied.
    pub fn ui(&mut self, ui: &mut egui::Ui, fb_tex: egui::TextureId, fb_lines: f32) -> egui::Rect {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::drag());
        if response.dragged() {
            let delta = response.drag_delta();
            self.camera.yaw -= delta.x * ORBIT_RADIANS_PER_POINT;
            self.camera.pitch =
                (self.camera.pitch + delta.y * ORBIT_RADIANS_PER_POINT).clamp(PITCH_MIN, PITCH_MAX);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.dist = (self.camera.dist * (-scroll * ZOOM_PER_SCROLL_POINT).exp())
                    .clamp(DIST_MIN, DIST_MAX);
            }
        }

        let props = Arc::clone(self.props.get_or_insert_with(|| Arc::new(build_props())));
        let view_proj = self
            .camera
            .view_proj(rect.width() / rect.height())
            .to_cols_array();
        let gl_state = Arc::clone(&self.gl_state);
        let snapshot = Arc::clone(&self.snapshot);
        let crt = self.crt.uniforms(fb_lines);
        let callback = eframe::egui_glow::CallbackFn::new(move |info, painter| {
            let gl = painter.gl();
            let mut state = gl_state.lock().unwrap();
            if matches!(*state, GlState::Uninit) {
                *state = match GlScene::new(gl, &props) {
                    Ok(scene) => GlState::Ready(scene),
                    Err(err) => {
                        tracing::error!("3D desk view disabled, GL setup failed: {err}");
                        GlState::Failed
                    }
                };
            }
            if let GlState::Ready(scene) = &mut *state {
                let vp = info.viewport_in_pixels();
                scene.paint(
                    gl,
                    &props,
                    &view_proj,
                    painter.texture(fb_tex),
                    crt,
                    painter.intermediate_fbo(),
                    [vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px],
                );
                let mut snapshot = snapshot.lock().unwrap();
                match snapshot.as_mut() {
                    Some((path, 0)) => {
                        save_snapshot(gl, &info, path);
                        *snapshot = None;
                    }
                    Some((_, frames_left)) => *frames_left -= 1,
                    None => {}
                }
            }
        });
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });
        rect
    }

    /// The "CRT" settings window (View → CRT Settings…): one toggle + one
    /// strength slider per shader pass, edited live.
    pub fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = self.show_settings;
        egui::Window::new(crate::window_title(ctx, "CRT"))
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                let crt = &mut self.crt;
                egui::Grid::new("crt_passes").num_columns(2).show(ui, |ui| {
                    for (on, strength, label, max) in [
                        (&mut crt.barrel_on, &mut crt.barrel, "Barrel curvature", 1.0),
                        (&mut crt.scanlines_on, &mut crt.scanlines, "Scanlines", 1.0),
                        (&mut crt.mask_on, &mut crt.mask, "Aperture grille", 1.0),
                        (
                            &mut crt.persistence_on,
                            &mut crt.persistence,
                            "Phosphor persistence",
                            PERSISTENCE_MAX,
                        ),
                        (&mut crt.bloom_on, &mut crt.bloom, "Bloom / bezel glow", 1.0),
                        (&mut crt.reflection_on, &mut crt.reflection, "Glass reflection", 1.0),
                    ] {
                        ui.checkbox(on, label);
                        ui.add_enabled(*on, egui::Slider::new(strength, 0.0..=max));
                        ui.end_row();
                    }
                });
                if ui.button("Reset to defaults").clicked() {
                    *crt = CrtParams::default();
                }
            });
        self.show_settings = open;
    }

    /// Free the GL resources (called from `CocoApp::on_exit`, the only hook
    /// eframe hands the `glow::Context` back to).
    pub fn destroy(&self, gl: Option<&glow::Context>) {
        let Some(gl) = gl else { return };
        let mut state = self.gl_state.lock().unwrap();
        if let GlState::Ready(scene) = &*state {
            scene.destroy(gl);
        }
        *state = GlState::Uninit;
    }
}
