//! CPU-side meshes and props.

use std::path::Path;

use glam::{Mat3, Mat4, Vec3};

use super::layout::{
    CART_COLOR, CART_SIZE, CASE_COLOR, CASE_SIZE, DESK_COLOR, DESK_SIZE, MONITOR_COLOR,
    MONITOR_SIZE, PropKind, SCREEN_HEIGHT, SCREEN_WIDTH, SWITCH_COLOR, SWITCH_SIZE, TV_COLOR,
    TV_SIZE,
};

/// Interleaved vertex data: position (3), normal (3), uv (2).
const FLOATS_PER_VERTEX: usize = 8;
pub(super) const VERTEX_STRIDE_BYTES: i32 = (FLOATS_PER_VERTEX * std::mem::size_of::<f32>()) as i32;

#[derive(Default)]
pub(super) struct MeshData {
    pub(super) verts: Vec<f32>,
    pub(super) indices: Vec<u32>,
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
pub(super) fn screen_quad() -> MeshData {
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

/// Object-space bounding box (from the mesh's vertices), used for picking.
#[derive(Clone, Copy)]
pub(super) struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    fn of(mesh: &MeshData) -> Self {
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for v in mesh.verts.chunks_exact(FLOATS_PER_VERTEX) {
            let p = Vec3::new(v[0], v[1], v[2]);
            min = min.min(p);
            max = max.max(p);
        }
        Self { min, max }
    }

    /// Slab test in object space; returns the entry distance along the ray.
    pub(super) fn hit(&self, origin: Vec3, dir: Vec3) -> Option<f32> {
        let inv = dir.recip();
        let t0 = (self.min - origin) * inv;
        let t1 = (self.max - origin) * inv;
        let (near, far) = (t0.min(t1), t0.max(t1));
        let t_enter = near.max_element();
        let t_exit = far.min_element();
        (t_enter <= t_exit && t_exit >= 0.0).then_some(t_enter.max(0.0))
    }
}

/// Static half of a prop: geometry and base color, uploaded to the GPU once.
/// Per-frame pose/highlight live in [`super::gl::Instance`].
pub(super) struct PropDef {
    pub(super) kind: PropKind,
    pub(super) mesh: MeshData,
    pub(super) color: [f32; 4],
    pub(super) aabb: Aabb,
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
pub(super) fn build_props() -> Vec<PropDef> {
    let assets = Path::new(ASSETS3D_DIR);
    let solid = |kind: PropKind, slug: &str, size: [f32; 3], color: [f32; 4]| {
        let mesh = load_gltf_mesh(&assets.join(format!("{slug}.glb")))
            .unwrap_or_else(|| cuboid(size));
        let aabb = Aabb::of(&mesh);
        PropDef {
            kind,
            mesh,
            color,
            aabb,
        }
    };
    let screen = screen_quad();
    let screen_aabb = Aabb::of(&screen);
    vec![
        solid(PropKind::Desk, "desk", DESK_SIZE, DESK_COLOR),
        solid(PropKind::MonitorCm8, "monitor-cm8", MONITOR_SIZE, MONITOR_COLOR),
        solid(PropKind::MonitorTv, "monitor-tv", TV_SIZE, TV_COLOR),
        solid(PropKind::Case, "coco3-case", CASE_SIZE, CASE_COLOR),
        solid(PropKind::PowerSwitch, "power-switch", SWITCH_SIZE, SWITCH_COLOR),
        solid(PropKind::Cartridge, "cartridge", CART_SIZE, CART_COLOR),
        PropDef {
            kind: PropKind::Screen,
            mesh: screen,
            color: [0.0, 0.0, 0.0, 1.0], // dark tube when off / no fb texture
            aabb: screen_aabb,
        },
    ]
}

/// View bytes of a `f32`/`u32` slice for `buffer_data_u8_slice` (safe: both
/// are plain-old-data with no padding).
pub(super) fn bytemuck_cast<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice))
    }
}
