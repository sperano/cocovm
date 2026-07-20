//! CPU-side mesh primitives, picking AABBs, and glTF loading. Prop
//! *composition* — which cuboids make a CoCo, a CM-8, a TV — lives in
//! `props.rs`.

use std::path::Path;

use glam::{Mat3, Mat4, Vec3};

use super::layout::PropKind;
use super::layout::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// Interleaved vertex data: position (3), normal (3), uv (2), color (3).
const FLOATS_PER_VERTEX: usize = 11;
pub(super) const VERTEX_STRIDE_BYTES: i32 = (FLOATS_PER_VERTEX * std::mem::size_of::<f32>()) as i32;
/// Attribute offsets, in floats, into one interleaved vertex.
pub(super) const OFFSET_NORMAL: i32 = 3;
pub(super) const OFFSET_UV: i32 = 6;
pub(super) const OFFSET_COLOR: i32 = 8;

#[derive(Default)]
pub(super) struct MeshData {
    pub(super) verts: Vec<f32>,
    pub(super) indices: Vec<u32>,
}

impl MeshData {
    fn push_vertex(&mut self, pos: [f32; 3], normal: [f32; 3], uv: [f32; 2], color: [f32; 3]) {
        self.verts.extend_from_slice(&pos);
        self.verts.extend_from_slice(&normal);
        self.verts.extend_from_slice(&uv);
        self.verts.extend_from_slice(&color);
    }

    /// Append an axis-aligned cuboid with per-face normals (flat shading) —
    /// the building block every procedural prop is composed from.
    pub(super) fn push_cuboid(&mut self, center: [f32; 3], size: [f32; 3], color: [f32; 3]) {
        let [cx, cy, cz] = center;
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
        for (normal, corners) in faces {
            let base = (self.verts.len() / FLOATS_PER_VERTEX) as u32;
            for corner in corners {
                let pos = [cx + corner[0], cy + corner[1], cz + corner[2]];
                self.push_vertex(pos, normal, [0.0, 0.0], color);
            }
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

/// The CRT face: a +Z-facing quad carrying the framebuffer texture. UV v = 0
/// at the top edge, matching egui's texture orientation (row 0 = top). White
/// vertex color: the fallback tint (`u_color`, black) does the darkening.
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
        mesh.push_vertex(pos, normal, uv, [1.0, 1.0, 1.0]);
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
    pub(super) fn of(mesh: &MeshData) -> Self {
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

/// Static half of a prop: geometry uploaded to the GPU once. Per-frame
/// pose/highlight live in [`super::gl::Instance`]; colors are baked into the
/// vertices.
pub(super) struct PropDef {
    pub(super) kind: PropKind,
    pub(super) mesh: MeshData,
    pub(super) aabb: Aabb,
}

/// Directory the modeled props load from, resolved relative to the process
/// working directory like `roms/` in the boot tests. Git-ignored.
pub(super) const ASSETS3D_DIR: &str = "assets3d";

/// Load a modeled prop, flattening the glTF node tree (transforms applied,
/// all primitives merged). Returns `None` — procedural prop — on any
/// missing/unreadable/empty file. Each primitive's vertices take its
/// material's base-color factor as their vertex color, so simple
/// solid-material models come through in their real colors (textures are
/// still ignored).
pub(super) fn load_gltf_mesh(path: &Path) -> Option<MeshData> {
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
            let base_color = prim
                .material()
                .pbr_metallic_roughness()
                .base_color_factor();
            let material_color = [base_color[0], base_color[1], base_color[2]];
            // Per-vertex COLOR_0 wins over the material factor — it's how
            // textured downloads come through after the Blender import
            // script bakes their texture to vertex colors
            // (docs/blender-props-guide.md).
            let vertex_colors: Vec<[f32; 3]> = reader
                .read_colors(0)
                .map(|c| c.into_rgb_f32().collect())
                .unwrap_or_default();
            let base = (out.verts.len() / FLOATS_PER_VERTEX) as u32;
            let mut vertex_count = 0u32;
            for (i, pos) in positions.enumerate() {
                let p = transform.transform_point3(Vec3::from(pos));
                let n = normals
                    .get(i)
                    .map(|n| (normal_matrix * Vec3::from(*n)).normalize_or_zero())
                    .unwrap_or(Vec3::Y);
                let color = vertex_colors.get(i).copied().unwrap_or(material_color);
                out.push_vertex([p.x, p.y, p.z], [n.x, n.y, n.z], [0.0, 0.0], color);
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

/// View bytes of a `f32`/`u32` slice for `buffer_data_u8_slice` (safe: both
/// are plain-old-data with no padding).
pub(super) fn bytemuck_cast<T: Copy>(slice: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), std::mem::size_of_val(slice))
    }
}
