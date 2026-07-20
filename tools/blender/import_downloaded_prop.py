"""Normalize a downloaded model (e.g. Sketchfab .glb) into a desk-view prop.

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --python tools/blender/import_downloaded_prop.py -- \
        --input assets3d/some_download.glb --slug coco3-case \
        [--yaw 180] [--width 0.38] [--bottom -0.04]

What it does:
- imports the model, joins all meshes, applies every transform;
- rotates by --yaw degrees around the up axis (fix a backwards-facing model
  by re-running with a different yaw — the app snapshot loop makes this a
  10-second iteration);
- uniformly rescales so the model's width (app X) equals --width, and
  recenters so X/depth are centered and the lowest point sits at --bottom
  (app-space Y, e.g. -0.04 = the CoCo case convention of resting on the
  desk when the prop transform places its center at case height);
- samples the material's base-color texture at every UV into a per-corner
  color attribute (the app's loader prefers COLOR_0 over material factors,
  and ignores textures), then strips the material so the heavy texture
  isn't re-exported;
- exports assets3d/<slug>.glb.

Per-slug defaults for --width/--bottom cover the props that exist today;
pass them explicitly for anything unusual.
"""

import argparse
import os
import sys

import bpy
import numpy as np

# (target width in app X, bottom in app Y) per slug — mirrors the prop
# conventions in crates/coco-egui/src/view3d/{layout,props}.rs.
SLUG_DEFAULTS = {
    "coco3-case": (0.38, -0.04),
    "monitor-cm8": (0.36, -0.17),
    "monitor-tv": (0.46, -0.19),
    "cartridge": (0.09, -0.0125),
    "power-switch": (0.030, -0.006),
    "desk": (1.60, -0.02),
}


def parse_args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    ap = argparse.ArgumentParser()
    ap.add_argument("--input", required=True)
    ap.add_argument("--slug", required=True)
    ap.add_argument("--yaw", type=float, default=0.0)
    ap.add_argument("--width", type=float)
    ap.add_argument("--bottom", type=float)
    args = ap.parse_args(argv)
    default_w, default_b = SLUG_DEFAULTS.get(args.slug, (None, None))
    if args.width is None:
        args.width = default_w
    if args.bottom is None:
        args.bottom = default_b
    if args.width is None or args.bottom is None:
        ap.error(f"unknown slug {args.slug!r}: pass --width and --bottom")
    return args


def join_imported_meshes():
    meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
    if not meshes:
        raise RuntimeError("no mesh objects in the imported file")
    bpy.ops.object.select_all(action="DESELECT")
    for obj in meshes:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.join()
    obj = bpy.context.active_object
    # Clear parents (Sketchfab wraps models in axis-fix empties) keeping the
    # world transform, then bake it into the mesh data.
    bpy.ops.object.parent_clear(type="CLEAR_KEEP_TRANSFORM")
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return obj


def normalize(obj, yaw_degrees, width, bottom):
    import math

    obj.rotation_euler = (0.0, 0.0, math.radians(yaw_degrees))
    bpy.ops.object.transform_apply(rotation=True)
    xs, ys, zs = zip(*[v.co for v in obj.data.vertices])
    scale = width / (max(xs) - min(xs))
    obj.scale = (scale, scale, scale)
    bpy.ops.object.transform_apply(scale=True)
    xs, ys, zs = zip(*[v.co for v in obj.data.vertices])
    # Blender: x = app x, y = app -z (depth), z = app y (up).
    dx = -(max(xs) + min(xs)) / 2.0
    dy = -(max(ys) + min(ys)) / 2.0
    dz = bottom - min(zs)
    for v in obj.data.vertices:
        v.co.x += dx
        v.co.y += dy
        v.co.z += dz


def find_base_color_image(obj):
    for slot in obj.material_slots:
        mat = slot.material
        if mat is None or not mat.use_nodes:
            continue
        for node in mat.node_tree.nodes:
            if node.type == "TEX_IMAGE" and node.image is not None:
                return node.image
    return None


def bake_texture_to_vertex_colors(obj):
    image = find_base_color_image(obj)
    mesh = obj.data
    attr = mesh.color_attributes.new("Col", "FLOAT_COLOR", "CORNER")
    loops = len(mesh.loops)
    colors = np.ones(loops * 4, dtype=np.float32)
    if image is not None and mesh.uv_layers.active is not None:
        w, h = image.size
        pixels = np.array(image.pixels[:], dtype=np.float32).reshape(h, w, 4)
        uvs = np.empty(loops * 2, dtype=np.float32)
        mesh.uv_layers.active.data.foreach_get("uv", uvs)
        uvs = uvs.reshape(loops, 2)
        px = (np.mod(uvs[:, 0], 1.0) * (w - 1)).astype(np.int32)
        py = (np.mod(uvs[:, 1], 1.0) * (h - 1)).astype(np.int32)
        colors = pixels[py, px].reshape(loops * 4).astype(np.float32)
        colors[3::4] = 1.0
    attr.data.foreach_set("color", colors)
    mesh.color_attributes.active_color = attr
    # Drop the materials: the app ignores textures, and stripping them keeps
    # the multi-megabyte texture out of the exported prop.
    obj.data.materials.clear()


def export(obj, path):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    kwargs = dict(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_apply=True,
    )
    try:
        bpy.ops.export_scene.gltf(**kwargs, export_vertex_color="ACTIVE")
    except TypeError:
        # Older exporter without the option: default behavior still writes
        # the active color attribute for unmaterialed meshes.
        bpy.ops.export_scene.gltf(**kwargs)


def main():
    args = parse_args()
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    out_path = os.path.join(repo, "assets3d", f"{args.slug}.glb")

    bpy.ops.wm.read_homefile(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=os.path.abspath(args.input))
    obj = join_imported_meshes()
    bake_texture_to_vertex_colors(obj)
    normalize(obj, args.yaw, args.width, args.bottom)
    obj.name = args.slug
    export(obj, out_path)
    print(
        f"wrote {out_path}: {len(obj.data.vertices)} verts, "
        f"width {args.width} m, bottom {args.bottom} m, yaw {args.yaw}°"
    )


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
