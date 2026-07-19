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

const FRAGMENT_SHADER: &str = r#"#version 150
uniform vec4 u_color;
uniform sampler2D u_tex;
uniform int u_use_tex;
in vec3 v_normal;
in vec2 v_uv;
out vec4 frag_color;
const vec3 LIGHT_DIR = vec3(0.35, 0.86, 0.37); // pre-normalized
const float AMBIENT = 0.35;
void main() {
    if (u_use_tex == 1) {
        // The CRT face is emissive: no lighting on the phosphor.
        frag_color = texture(u_tex, v_uv);
    } else {
        float diffuse = max(dot(normalize(v_normal), LIGHT_DIR), 0.0);
        float light = mix(AMBIENT, 1.0, diffuse);
        frag_color = vec4(u_color.rgb * light, u_color.a);
    }
}
"#;

struct GlMesh {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    ebo: glow::Buffer,
    index_count: i32,
}

struct GlScene {
    program: glow::Program,
    u_view_proj: Option<glow::UniformLocation>,
    u_model: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    u_use_tex: Option<glow::UniformLocation>,
    /// One entry per prop, same order as the `PropDef` list.
    meshes: Vec<GlMesh>,
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

    fn new(gl: &glow::Context, props: &[PropDef]) -> Result<Self, String> {
        unsafe {
            let vs = Self::compile_shader(gl, glow::VERTEX_SHADER, VERTEX_SHADER)?;
            let fs = Self::compile_shader(gl, glow::FRAGMENT_SHADER, FRAGMENT_SHADER)?;
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

            let mut meshes = Vec::with_capacity(props.len());
            for prop in props {
                let vao = gl.create_vertex_array()?;
                let vbo = gl.create_buffer()?;
                let ebo = gl.create_buffer()?;
                gl.bind_vertex_array(Some(vao));
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                gl.buffer_data_u8_slice(
                    glow::ARRAY_BUFFER,
                    bytemuck_cast(&prop.mesh.verts),
                    glow::STATIC_DRAW,
                );
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
                gl.buffer_data_u8_slice(
                    glow::ELEMENT_ARRAY_BUFFER,
                    bytemuck_cast(&prop.mesh.indices),
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
                meshes.push(GlMesh {
                    vao,
                    vbo,
                    ebo,
                    index_count: prop.mesh.indices.len() as i32,
                });
            }

            // The framebuffer sampler always reads texture unit 0.
            gl.use_program(Some(program));
            if let Some(loc) = gl.get_uniform_location(program, "u_tex") {
                gl.uniform_1_i32(Some(&loc), 0);
            }

            Ok(Self {
                u_view_proj: gl.get_uniform_location(program, "u_view_proj"),
                u_model: gl.get_uniform_location(program, "u_model"),
                u_color: gl.get_uniform_location(program, "u_color"),
                u_use_tex: gl.get_uniform_location(program, "u_use_tex"),
                program,
                meshes,
            })
        }
    }

    /// Draw the whole prop list. Runs inside the paint callback: viewport is
    /// already the panel rect and scissor already clips to it, so the clear
    /// only touches our pixels; egui_glow restores its own state afterwards.
    fn paint(
        &self,
        gl: &glow::Context,
        props: &[PropDef],
        view_proj: &[f32; 16],
        fb_texture: Option<glow::Texture>,
    ) {
        unsafe {
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
            for (prop, mesh) in props.iter().zip(&self.meshes) {
                let use_tex = prop.is_screen && fb_texture.is_some();
                if use_tex {
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, fb_texture);
                }
                gl.uniform_matrix_4_f32_slice(
                    self.u_model.as_ref(),
                    false,
                    &prop.transform.to_cols_array(),
                );
                gl.uniform_4_f32_slice(self.u_color.as_ref(), &prop.color);
                gl.uniform_1_i32(self.u_use_tex.as_ref(), use_tex as i32);
                gl.bind_vertex_array(Some(mesh.vao));
                gl.draw_elements(glow::TRIANGLES, mesh.index_count, glow::UNSIGNED_INT, 0);
            }
            gl.bind_vertex_array(None);
        }
    }

    fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            for mesh in &self.meshes {
                gl.delete_vertex_array(mesh.vao);
                gl.delete_buffer(mesh.vbo);
                gl.delete_buffer(mesh.ebo);
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
    /// per-frame `coco-fb` texture (uploaded by `step_emulation`). Dragging
    /// orbits, scrolling zooms. Returns the rect it occupied.
    pub fn ui(&mut self, ui: &mut egui::Ui, fb_tex: egui::TextureId) -> egui::Rect {
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
            if let GlState::Ready(scene) = &*state {
                scene.paint(gl, &props, &view_proj, painter.texture(fb_tex));
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
