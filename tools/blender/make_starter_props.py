"""Starter Blender scene + .glb export for the 3D desk view's props.

Run headless (regenerates every prop .glb and the editable .blend):

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --python tools/blender/make_starter_props.py

or open ``assets3d/props-starter.blend`` in Blender, edit, and export any
one collection with File > Export > glTF 2.0 (settings in
``docs/blender-props-guide.md``).

One collection per prop, one object per part, sized in real meters. The
part list mirrors the procedural props in
``crates/coco-egui/src/view3d/props.rs`` — same shapes, same colors — so
the exported models start visually identical to the in-app fallbacks and
every edit in Blender is pure improvement. Main shells get a small bevel
modifier (applied on export), which is already something the procedural
cuboids can't do.

Coordinate convention (docs/blender-props-guide.md): the app's prop space
is x-right / y-up / z-toward-viewer; Blender is z-up with the front view
looking at -Y. The glTF exporter's default +Y-up conversion maps Blender
(x, y, z) -> app (x, z, -y), so this script places parts at
(x, -z_app, y_app) and everything lands where the app expects it.
"""

import os
import sys

import bpy

# --- part tables (mirror crates/coco-egui/src/view3d/props.rs) -----------

BEIGE = (0.82, 0.80, 0.75)
MONITOR_SHELL = (0.75, 0.73, 0.68)
CM8_BEZEL = (0.66, 0.64, 0.60)
TUBE_SURROUND = (0.05, 0.05, 0.05)
KNOB = (0.30, 0.30, 0.30)
WALNUT = (0.30, 0.20, 0.13)
TV_PANEL = (0.15, 0.14, 0.13)
TV_KNOB = (0.65, 0.65, 0.66)
KEY = (0.36, 0.35, 0.34)
KEY_TRAY = (0.20, 0.20, 0.20)
LED = (0.80, 0.12, 0.10)
BADGE = (0.55, 0.55, 0.58)
DESK_TOP = (0.45, 0.32, 0.22)
LEG = (0.30, 0.20, 0.13)
FLOOR = (0.20, 0.17, 0.15)
CART_BODY = (0.25, 0.25, 0.27)
LABEL = (0.80, 0.76, 0.68)
RIDGE = (0.18, 0.18, 0.20)
SWITCH = (0.35, 0.35, 0.36)

# (name, center_app, size_app, color, bevel_width_or_None)
SHELL_BEVEL = 0.004


def case_parts():
    parts = [
        ("base", (0, -0.025, 0), (0.38, 0.03, 0.27), BEIGE, SHELL_BEVEL),
        ("rear-deck", (0, 0.015, -0.08), (0.38, 0.05, 0.11), BEIGE, SHELL_BEVEL),
        ("key-tray", (0, -0.004, 0.0525), (0.31, 0.012, 0.115), KEY_TRAY, None),
        ("led", (0.16, 0.042, -0.05), (0.012, 0.004, 0.008), LED, None),
        ("badge", (-0.14, 0.041, -0.04), (0.05, 0.002, 0.012), BADGE, None),
        ("spacebar", (0, 0.007, 0.093), (0.14, 0.010, 0.018), KEY, None),
    ]
    for row in range(3):
        z = 0.015 + 0.026 * row
        for col in range(13):
            x = 0.0225 * (col - 6)
            parts.append((f"key-r{row}c{col}", (x, 0.007, z), (0.018, 0.010, 0.018), KEY, None))
    return parts


def tv_parts():
    parts = [
        ("cabinet", (0, 0.01, -0.02), (0.46, 0.36, 0.40), WALNUT, SHELL_BEVEL),
        ("panel-top", (0, 0.145, 0.195), (0.42, 0.05, 0.05), TV_PANEL, None),
        ("panel-bottom", (0, -0.125, 0.195), (0.42, 0.05, 0.05), TV_PANEL, None),
        ("panel-left", (-0.175, 0.01, 0.195), (0.07, 0.22, 0.05), TV_PANEL, None),
        ("panel-right", (0.175, 0.01, 0.195), (0.07, 0.22, 0.05), TV_PANEL, None),
        ("tube-surround", (0, 0.01, 0.16), (0.30, 0.24, 0.01), TUBE_SURROUND, None),
        ("knob-top", (0.175, 0.075, 0.225), (0.03, 0.03, 0.015), TV_KNOB, None),
        ("knob-bottom", (0.175, 0.015, 0.225), (0.03, 0.03, 0.015), TV_KNOB, None),
    ]
    for x in (-0.19, 0.19):
        for z in (-0.15, 0.11):
            parts.append((f"foot-{x:+.2f}-{z:+.2f}", (x, -0.18, z), (0.05, 0.02, 0.05), RIDGE, None))
    for i in range(3):
        y = -0.055 - 0.015 * i
        parts.append((f"speaker-slit-{i}", (0.175, y, 0.222), (0.05, 0.005, 0.006), RIDGE, None))
    return parts


def desk_parts():
    parts = [
        ("top", (0, 0, 0), (1.60, 0.04, 0.90), DESK_TOP, SHELL_BEVEL),
        ("floor", (0, -0.71, 0), (3.5, 0.02, 3.0), FLOOR, None),
    ]
    for x in (-0.72, 0.72):
        for z in (-0.38, 0.38):
            parts.append((f"leg-{x:+.2f}-{z:+.2f}", (x, -0.36, z), (0.06, 0.68, 0.06), LEG, None))
    return parts


def cartridge_parts():
    parts = [
        ("body", (0, 0, 0), (0.09, 0.025, 0.13), CART_BODY, 0.002),
        ("label", (0, 0.0135, 0.015), (0.072, 0.002, 0.080), LABEL, None),
    ]
    for i in range(3):
        z = -0.045 - 0.008 * i
        parts.append((f"ridge-{i}", (0, 0.010, z), (0.09, 0.004, 0.005), RIDGE, None))
    return parts


PROPS = {
    "desk": desk_parts(),
    "monitor-cm8": [
        ("shell", (0, 0, -0.04), (0.36, 0.34, 0.30), MONITOR_SHELL, SHELL_BEVEL),
        ("bezel-top", (0, 0.155, 0.15), (0.36, 0.03, 0.08), CM8_BEZEL, None),
        ("bezel-bottom", (0, -0.125, 0.15), (0.36, 0.09, 0.08), CM8_BEZEL, None),
        ("bezel-left", (-0.16, 0.03, 0.15), (0.04, 0.22, 0.08), CM8_BEZEL, None),
        ("bezel-right", (0.16, 0.03, 0.15), (0.04, 0.22, 0.08), CM8_BEZEL, None),
        ("tube-surround", (0, 0.03, 0.105), (0.30, 0.24, 0.01), TUBE_SURROUND, None),
        ("knob-left", (0.08, -0.125, 0.196), (0.02, 0.02, 0.012), KNOB, None),
        ("knob-right", (0.13, -0.125, 0.196), (0.02, 0.02, 0.012), KNOB, None),
        ("power-led", (-0.13, -0.125, 0.196), (0.012, 0.006, 0.008), LED, None),
    ],
    "monitor-tv": tv_parts(),
    "coco3-case": case_parts(),
    "power-switch": [
        ("button", (0, 0, 0), (0.030, 0.012, 0.025), SWITCH, 0.002),
    ],
    "cartridge": cartridge_parts(),
}

# --- scene building -------------------------------------------------------


def app_to_blender(center, size):
    """App prop space (x right, y up, z front) -> Blender (z up, -Y front)."""
    cx, cy, cz = center
    sx, sy, sz = size
    return (cx, -cz, cy), (sx, sz, sy)


def material(name, rgb):
    mat = bpy.data.materials.get(name)
    if mat is None:
        mat = bpy.data.materials.new(name)
        mat.use_nodes = True
        bsdf = mat.node_tree.nodes.get("Principled BSDF")
        bsdf.inputs["Base Color"].default_value = (*rgb, 1.0)
        mat.diffuse_color = (*rgb, 1.0)  # viewport solid-mode color
    return mat


def build_prop(slug, parts):
    collection = bpy.data.collections.new(slug)
    bpy.context.scene.collection.children.link(collection)
    for name, center, size, color, bevel in parts:
        loc, dim = app_to_blender(center, size)
        bpy.ops.mesh.primitive_cube_add(size=1.0, location=loc)
        obj = bpy.context.active_object
        obj.name = f"{slug}/{name}"
        obj.dimensions = dim
        obj.data.materials.append(material(f"mat-{color}", color))
        if bevel is not None:
            mod = obj.modifiers.new("Bevel", "BEVEL")
            mod.width = bevel
            mod.segments = 2
        for other in list(obj.users_collection):
            other.objects.unlink(obj)
        collection.objects.link(obj)
    return collection


def export_collection(collection, path):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in collection.objects:
        obj.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_apply=True,  # bake the bevel modifiers into the export
    )


def main():
    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    assets = os.path.join(repo, "assets3d")
    os.makedirs(assets, exist_ok=True)

    bpy.ops.wm.read_homefile(use_empty=True)
    for slug, parts in PROPS.items():
        build_prop(slug, parts)
    for collection in list(bpy.context.scene.collection.children):
        export_collection(collection, os.path.join(assets, f"{collection.name}.glb"))
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(assets, "props-starter.blend"))
    print(f"wrote {len(PROPS)} .glb props + props-starter.blend to {assets}")


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:  # non-zero exit so a failed run is visible
        print(f"FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
