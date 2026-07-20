# Blender guide: modeling props for the 3D desk view

You do not start from a blank scene. `assets3d/props-starter.blend` contains
every prop as a correctly-sized, correctly-placed blockout (regenerate it any
time with the command below). Modeling here means *refining* those boxes —
bevel an edge, carve a vent, round the tube — and re-exporting. The app
falls back to its built-in procedural props for any `.glb` that's missing,
so nothing ever breaks.

## The loop

1. Open the scene: `open -a Blender assets3d/props-starter.blend`
2. Edit a prop (see "first session" below).
3. Export it: select the prop's objects (click its collection, `A` won't do —
   select the objects), then File → Export → glTF 2.0 into
   `assets3d/<slug>.glb`. Settings that matter:
   - Format: **glTF Binary (.glb)**
   - Include → **Selected Objects** ✓
   - Data → Mesh → **Apply Modifiers** ✓ (bakes bevels etc.)
   - Transform → **+Y Up** ✓ (the default — leave it)
4. See it live: `cargo run -p coco-egui -- --view-3d`
   (or headless proof: `COCO_VIEW3D_SNAPSHOT=/tmp/desk.png` before the
   command writes a PNG of the rendered desk after a few seconds).

To regenerate everything from the tables in the script (wipes your edits —
the `.blend` is your working file, keep it):

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --python tools/blender/make_starter_props.py

## Conventions (the script already obeys all of these)

- **Units are meters**, real hardware sizes (CoCo 3 case ≈ 0.38 m wide).
- **Origin = prop center.** The app places each prop by its own transform;
  your model just needs to be centered where the blockout is.
- **Front faces -Y in Blender** (the direction Front view — numpad 1 —
  looks at). The exporter's +Y-Up conversion turns that into the app's
  "front toward the camera".
- **Solid materials only.** The loader bakes each primitive's Principled
  BSDF **Base Color** into vertex colors. Textures/images are ignored (for
  now) — use one material per colored part, like the starter scene does.
- **File names** are fixed: `desk`, `monitor-cm8`, `monitor-tv`,
  `coco3-case`, `power-switch`, `cartridge` (+ `.glb`), in `assets3d/`
  (git-ignored — local like `roms/`).
- **Keep the tube plane.** Both monitors' front faces sit in one plane (the
  screen quad and bezel glow live there): local z = +0.19 for the CM-8,
  +0.22 for the TV, with a bezel opening of about 0.28 × 0.22 around the
  glass (screen center: y = +0.03 in CM-8 space, +0.01 in TV space). Reshape
  the shells freely; keep the opening around the glass.
- **Picking uses the mesh's bounding box**, so wildly oversized decorative
  geometry on an interactive prop (switch, cartridge, monitors) enlarges
  its click target.

## First session, concrete (15 minutes, makes the CM-8 look real)

1. Open `props-starter.blend`. In the Outliner (top right), click the
   `monitor-cm8` collection's arrow and click `monitor-cm8/shell`.
2. `Tab` → Edit Mode. `2` for edge select. Hover a vertical front edge,
   `Alt+Click` to select the edge loop.
3. `Ctrl+B`, drag a little, scroll wheel to add segments → beveled corner.
   `Tab` back to Object Mode.
4. Rounder still: with the shell selected, Modifier Properties (wrench
   icon) → the starter Bevel modifier is already there — raise its Width
   to ~0.008 and Segments to 3.
5. CRT tubes aren't flat: select `monitor-cm8/tube-surround`, `Tab`,
   `3` (face select), pick the front face, `I` (inset) then `Alt+E` →
   Extrude Along Normals slightly for a curved-glass hint.
6. Select all `monitor-cm8/*` objects (click first, `Shift`-click rest, or
   drag a box in the viewport), export per step 3 above, run the app.

Small, incremental, always-working — same philosophy as the emulator.

## Ideas per prop, roughly in payoff order

- `monitor-cm8`: bevel the shell; slight wedge (narrower at back) — select
  the back face in Edit Mode and `S`cale it to ~0.9; vent slots on top.
- `coco3-case`: the real CoCo 3 is a wedge — in Edit Mode grab the rear
  deck's top-front edge loop and pull it down/forward; key caps with a tiny
  bevel read instantly as a keyboard.
- `monitor-tv`: round the cabinet corners generously (70s sets were soft);
  a fabric-look speaker area from a subdivided inset.
- `cartridge`: bevel the leading edge, thin the grip end.
- `desk`: edge profile on the top slab; simple trestle legs.

Reference photos: your real 2048K machine, and the manual scans the manager
already ships in `images/` (`paths::images_dir()`).

## Downloaded models (Sketchfab etc.)

`tools/blender/import_downloaded_prop.py` normalizes a downloaded `.glb`
into a prop: joins meshes, bakes its base-color texture into vertex colors
(the app ignores textures but prefers COLOR_0), rescales/recenters to the
prop conventions, strips the heavy texture, exports `assets3d/<slug>.glb`:

    /Applications/Blender.app/Contents/MacOS/Blender --background \
        --python tools/blender/import_downloaded_prop.py -- \
        --input assets3d/some_download.glb --slug coco3-case [--yaw 180]

If the model faces the wrong way, re-run with `--yaw`; the snapshot loop
makes that a quick iteration.

**Licensing:** `assets3d/` is git-ignored, but keep the attribution for
anything downloaded here, in case a prop ever ships in the asset tarball:

- `coco3-case.glb` (in use): "TRS-80 Color Computer 2"
  (https://skfb.ly/6V6IL) by ericomont, licensed CC Attribution 4.0
  (http://creativecommons.org/licenses/by/4.0/). Source download kept as
  `assets3d/trs-80_color_computer_2.glb`.
