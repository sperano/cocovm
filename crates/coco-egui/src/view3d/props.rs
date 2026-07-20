//! Procedural prop composition: which cuboids make a desk, a CoCo 3, a
//! CM-8, a TV, a pak. Everything is flat-shaded boxes with baked vertex
//! colors — deliberately stylized, so nobody has to model anything in
//! Blender; a real `assets3d/<slug>.glb` still overrides any prop it
//! exists for. All geometry is in each prop's object space (the same
//! centered convention the placeholder boxes used), so poses and picking
//! AABBs are unchanged.

use std::path::Path;

use super::layout::{
    CART_COLOR, CART_SIZE, CASE_COLOR, CASE_SIZE, DESK_COLOR, DESK_SIZE, MONITOR_COLOR,
    MONITOR_SIZE, PropKind, SWITCH_COLOR, SWITCH_SIZE, TV_COLOR, TV_SIZE,
};
use super::mesh::{ASSETS3D_DIR, Aabb, MeshData, PropDef, load_gltf_mesh, screen_quad};

/// Bezel/frame tone: slightly darker than the CM-8 shell for depth.
const CM8_BEZEL_COLOR: [f32; 3] = [0.66, 0.64, 0.60];
/// The dark interior visible through both monitors' bezel openings, just
/// behind the glass.
const TUBE_SURROUND_COLOR: [f32; 3] = [0.05, 0.05, 0.05];
const KNOB_COLOR: [f32; 3] = [0.30, 0.30, 0.30];
const TV_PANEL_COLOR: [f32; 3] = [0.15, 0.14, 0.13];
const TV_KNOB_COLOR: [f32; 3] = [0.65, 0.65, 0.66];
const KEY_COLOR: [f32; 3] = [0.36, 0.35, 0.34];
const KEY_TRAY_COLOR: [f32; 3] = [0.20, 0.20, 0.20];
const POWER_LED_COLOR: [f32; 3] = [0.80, 0.12, 0.10];
const BADGE_COLOR: [f32; 3] = [0.55, 0.55, 0.58];
const LABEL_COLOR: [f32; 3] = [0.80, 0.76, 0.68];
const RIDGE_COLOR: [f32; 3] = [0.18, 0.18, 0.20];
const LEG_COLOR: [f32; 3] = [0.30, 0.20, 0.13];
const FLOOR_COLOR: [f32; 3] = [0.20, 0.17, 0.15];

fn rgb(color: [f32; 4]) -> [f32; 3] {
    [color[0], color[1], color[2]]
}

/// Desk slab (centered like the old placeholder — the world transform puts
/// its top at y = 0) plus legs and a floor slab far below, so the scene
/// isn't a slab floating in the void.
fn desk() -> MeshData {
    let mut m = MeshData::default();
    m.push_cuboid([0.0, 0.0, 0.0], DESK_SIZE, rgb(DESK_COLOR));
    for x in [-0.72, 0.72] {
        for z in [-0.38, 0.38] {
            m.push_cuboid([x, -0.36, z], [0.06, 0.68, 0.06], LEG_COLOR);
        }
    }
    m.push_cuboid([0.0, -0.71, 0.0], [3.5, 0.02, 3.0], FLOOR_COLOR);
    m
}

/// CM-8: beige shell, recessed bezel frame around the tube opening, dark
/// interior behind the glass, two knobs and a power light on the wide
/// bottom bezel. Front face at local z = +0.19 (the shared tube plane).
fn monitor_cm8() -> MeshData {
    let mut m = MeshData::default();
    let shell = rgb(MONITOR_COLOR);
    m.push_cuboid([0.0, 0.0, -0.04], [MONITOR_SIZE[0], MONITOR_SIZE[1], 0.30], shell);
    // Bezel frame (z 0.11..0.19) around a 0.28 × 0.22 opening centered at
    // y = +0.03 — the screen quad's height in this prop's space.
    m.push_cuboid([0.0, 0.155, 0.15], [0.36, 0.03, 0.08], CM8_BEZEL_COLOR);
    m.push_cuboid([0.0, -0.125, 0.15], [0.36, 0.09, 0.08], CM8_BEZEL_COLOR);
    m.push_cuboid([-0.16, 0.03, 0.15], [0.04, 0.22, 0.08], CM8_BEZEL_COLOR);
    m.push_cuboid([0.16, 0.03, 0.15], [0.04, 0.22, 0.08], CM8_BEZEL_COLOR);
    m.push_cuboid([0.0, 0.03, 0.105], [0.30, 0.24, 0.01], TUBE_SURROUND_COLOR);
    m.push_cuboid([0.08, -0.125, 0.196], [0.02, 0.02, 0.012], KNOB_COLOR);
    m.push_cuboid([0.13, -0.125, 0.196], [0.02, 0.02, 0.012], KNOB_COLOR);
    m.push_cuboid([-0.13, -0.125, 0.196], [0.012, 0.006, 0.008], POWER_LED_COLOR);
    m
}

/// Consumer TV: walnut cabinet on feet, charcoal front panel with the tube
/// opening off to the left of a control column (two silver dials, speaker
/// slits below). Front face at local z = +0.22, same tube plane as the CM-8.
fn monitor_tv() -> MeshData {
    let mut m = MeshData::default();
    m.push_cuboid([0.0, 0.01, -0.02], [TV_SIZE[0], 0.36, 0.40], rgb(TV_COLOR));
    for x in [-0.19, 0.19] {
        for z in [-0.15, 0.11] {
            m.push_cuboid([x, -0.18, z], [0.05, 0.02, 0.05], RIDGE_COLOR);
        }
    }
    // Front panel (z 0.17..0.22) around a 0.28 × 0.22 opening centered at
    // y = +0.01 (the screen height in TV space).
    m.push_cuboid([0.0, 0.145, 0.195], [0.42, 0.05, 0.05], TV_PANEL_COLOR);
    m.push_cuboid([0.0, -0.125, 0.195], [0.42, 0.05, 0.05], TV_PANEL_COLOR);
    m.push_cuboid([-0.175, 0.01, 0.195], [0.07, 0.22, 0.05], TV_PANEL_COLOR);
    m.push_cuboid([0.175, 0.01, 0.195], [0.07, 0.22, 0.05], TV_PANEL_COLOR);
    m.push_cuboid([0.0, 0.01, 0.16], [0.30, 0.24, 0.01], TUBE_SURROUND_COLOR);
    m.push_cuboid([0.175, 0.075, 0.225], [0.03, 0.03, 0.015], TV_KNOB_COLOR);
    m.push_cuboid([0.175, 0.015, 0.225], [0.03, 0.03, 0.015], TV_KNOB_COLOR);
    for i in 0..3 {
        let y = -0.055 - 0.015 * i as f32;
        m.push_cuboid([0.175, y, 0.222], [0.05, 0.005, 0.006], RIDGE_COLOR);
    }
    m
}

/// CoCo 3 case: low front half carrying the recessed keyboard, raised rear
/// deck, power LED. The keyboard is a grid of key caps over a dark tray —
/// stylized, not a keymap.
fn coco3_case() -> MeshData {
    let mut m = MeshData::default();
    let shell = rgb(CASE_COLOR);
    m.push_cuboid([0.0, -0.025, 0.0], [CASE_SIZE[0], 0.03, CASE_SIZE[2]], shell);
    m.push_cuboid([0.0, 0.015, -0.08], [CASE_SIZE[0], 0.05, 0.11], shell);
    m.push_cuboid([0.0, -0.004, 0.0525], [0.31, 0.012, 0.115], KEY_TRAY_COLOR);
    const KEY_SIZE: [f32; 3] = [0.018, 0.010, 0.018];
    const KEY_PITCH_X: f32 = 0.0225;
    const KEY_PITCH_Z: f32 = 0.026;
    const KEY_COLS: i32 = 13;
    for row in 0..3 {
        let z = 0.015 + KEY_PITCH_Z * row as f32;
        for col in 0..KEY_COLS {
            let x = KEY_PITCH_X * (col - KEY_COLS / 2) as f32;
            m.push_cuboid([x, 0.007, z], KEY_SIZE, KEY_COLOR);
        }
    }
    m.push_cuboid([0.0, 0.007, 0.015 + KEY_PITCH_Z * 3.0], [0.14, 0.010, 0.018], KEY_COLOR);
    m.push_cuboid([0.16, 0.042, -0.05], [0.012, 0.004, 0.008], POWER_LED_COLOR);
    m.push_cuboid([-0.14, 0.041, -0.04], [0.05, 0.002, 0.012], BADGE_COLOR);
    m
}

/// ROM pak: dark shell, lighter label patch on top, grip ridges at the
/// outer end.
fn cartridge() -> MeshData {
    let mut m = MeshData::default();
    m.push_cuboid([0.0, 0.0, 0.0], CART_SIZE, rgb(CART_COLOR));
    m.push_cuboid([0.0, 0.0135, 0.015], [0.072, 0.002, 0.080], LABEL_COLOR);
    for i in 0..3 {
        let z = -0.045 - 0.008 * i as f32;
        m.push_cuboid([0.0, 0.010, z], [CART_SIZE[0], 0.004, 0.005], RIDGE_COLOR);
    }
    m
}

fn power_switch() -> MeshData {
    let mut m = MeshData::default();
    m.push_cuboid([0.0, 0.0, 0.0], SWITCH_SIZE, rgb(SWITCH_COLOR));
    m
}

/// The whole prop set, in draw order. Each solid prop tries
/// `assets3d/<slug>.glb` first (real colors via material base-color
/// factors); the screen quad is always generated (it carries the
/// framebuffer and must keep its UV mapping).
pub(super) fn build_props() -> Vec<PropDef> {
    let assets = Path::new(ASSETS3D_DIR);
    let prop = |kind: PropKind, slug: &str, procedural: fn() -> MeshData| {
        let mesh =
            load_gltf_mesh(&assets.join(format!("{slug}.glb"))).unwrap_or_else(procedural);
        let aabb = Aabb::of(&mesh);
        PropDef { kind, mesh, aabb }
    };
    let screen = screen_quad();
    let screen_aabb = Aabb::of(&screen);
    vec![
        prop(PropKind::Desk, "desk", desk),
        prop(PropKind::MonitorCm8, "monitor-cm8", monitor_cm8),
        prop(PropKind::MonitorTv, "monitor-tv", monitor_tv),
        prop(PropKind::Case, "coco3-case", coco3_case),
        prop(PropKind::PowerSwitch, "power-switch", power_switch),
        prop(PropKind::Cartridge, "cartridge", cartridge),
        PropDef {
            kind: PropKind::Screen,
            mesh: screen,
            aabb: screen_aabb,
        },
    ]
}
