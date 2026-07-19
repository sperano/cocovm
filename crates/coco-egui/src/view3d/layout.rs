//! Desk layout. All dimensions in meters, placeholder-box sizes roughly
//! matching the real hardware; replaced visually (not positionally) once a
//! modeled `.glb` exists for the prop.

use glam::{Mat4, Vec3};

use super::crt::{GLOW_QUAD_LIFT, GLOW_QUAD_SCALE};

/// Desk top slab: width, thickness, depth. Top surface sits at y = 0.
pub(super) const DESK_SIZE: [f32; 3] = [1.60, 0.04, 0.90];
/// CoCo 3 case shell: width, height, depth.
pub(super) const CASE_SIZE: [f32; 3] = [0.38, 0.08, 0.27];
/// CM-8 monitor shell (13" analog-RGB CRT, see [[desk-scene-verified-facts]]
/// in the session notes / plan doc): width, height, depth.
pub(super) const MONITOR_SIZE: [f32; 3] = [0.36, 0.34, 0.38];
/// TV shell (composite/RF alternative): a bigger, deeper consumer set. Sized
/// so its FRONT FACE sits in the same plane as the CM-8's — the screen quad
/// and bezel glow stay put when the monitor is swapped.
pub(super) const TV_SIZE: [f32; 3] = [0.46, 0.38, 0.44];
/// A ROM pak lying on the desk: width, height, depth.
pub(super) const CART_SIZE: [f32; 3] = [0.09, 0.025, 0.13];
/// Visible CRT glass width; height is fixed 4:3 like the real tube, so the
/// framebuffer stretches to the monitor's aspect exactly as `aspect_correct`
/// does in the flat view.
pub(super) const SCREEN_WIDTH: f32 = 0.26;
pub(super) const SCREEN_HEIGHT: f32 = SCREEN_WIDTH * 3.0 / 4.0;
/// Lift of the screen quad off the monitor's front face, avoiding z-fighting.
const SCREEN_LIFT: f32 = 0.002;

/// Monitor center: behind the case, sitting on the desk.
pub(super) const MONITOR_POS: [f32; 3] = [0.0, MONITOR_SIZE[1] / 2.0, -0.15];
/// The shared front-face plane both monitor shells present the tube in.
const MONITOR_FRONT_Z: f32 = MONITOR_POS[2] + MONITOR_SIZE[2] / 2.0;
/// TV center: same front plane as the CM-8, extra depth grows backwards.
pub(super) const TV_POS: [f32; 3] = [0.0, TV_SIZE[1] / 2.0, MONITOR_FRONT_Z - TV_SIZE[2] / 2.0];
/// Screen center: on the monitor's front face, slightly above monitor center
/// (real CRTs put the tube high, electronics below).
pub(super) const SCREEN_POS: [f32; 3] = [
    0.0,
    MONITOR_POS[1] + 0.03,
    MONITOR_POS[2] + MONITOR_SIZE[2] / 2.0 + SCREEN_LIFT,
];
/// Case center: in front of the monitor, on the desk.
pub(super) const CASE_POS: [f32; 3] = [0.0, CASE_SIZE[1] / 2.0, 0.16];
/// Cartridge center: on the desk to the right of the case.
const CART_POS: [f32; 3] = [0.42, CART_SIZE[1] / 2.0, 0.10];
/// Cartridge yaw (radians) so it doesn't sit unnaturally axis-aligned.
const CART_YAW: f32 = -0.4;

/// Power switch button: width, height, depth — sits on top of the case
/// toward the right rear. Placement is approximate until checked against
/// case photos / the service manual; it's cosmetic, not a hardware claim.
pub(super) const SWITCH_SIZE: [f32; 3] = [0.030, 0.012, 0.025];
pub(super) const SWITCH_POS: [f32; 3] = [
    CASE_POS[0] + 0.15,
    CASE_SIZE[1] + SWITCH_SIZE[1] / 2.0,
    CASE_POS[2] - 0.09,
];
/// The switch sinks by this much when the machine is off (pushed in).
pub(super) const SWITCH_PRESSED_OFFSET: f32 = -0.005;
/// Test-only re-exports of pick-target centers (`ui_tests` aims clicks here).
#[cfg(test)]
pub(crate) const SWITCH_WORLD: [f32; 3] = SWITCH_POS;
#[cfg(test)]
pub(crate) const CART_SLOT_WORLD: [f32; 3] = CART_SLOT_POS;
/// A point on the monitor bezel — right of the screen glass, on the shared
/// front face — so the swap test hits the shell, not the (occluding) screen.
#[cfg(test)]
pub(crate) const MONITOR_BEZEL_WORLD: [f32; 3] = [
    (SCREEN_WIDTH / 2.0 + MONITOR_SIZE[0] / 2.0) / 2.0,
    SCREEN_POS[1],
    MONITOR_FRONT_Z,
];

/// Cartridge-slot pose: the CoCo's cartridge port is on the right side of
/// the case; an inserted pak sticks out of that side, long axis along X.
const CART_SLOT_YAW: f32 = std::f32::consts::FRAC_PI_2;
/// How deep the pak sits inside the case when inserted.
const CART_INSERT_DEPTH: f32 = 0.05;
const CART_SLOT_POS: [f32; 3] = [
    CASE_SIZE[0] / 2.0 - CART_INSERT_DEPTH + CART_SIZE[2] / 2.0,
    CASE_POS[1],
    CASE_POS[2],
];
/// Insert/eject slide duration. The emulator call fires only when the
/// animation completes — the physical layer is a veneer over the same
/// operations the menus use (plan decision 2).
pub(super) const CART_ANIM_SECS: f32 = 0.35;
/// Peak of the little arc the pak travels through while animating.
const CART_ANIM_ARC: f32 = 0.06;

pub(super) const DESK_COLOR: [f32; 4] = [0.45, 0.32, 0.22, 1.0];
pub(super) const CASE_COLOR: [f32; 4] = [0.82, 0.80, 0.75, 1.0];
pub(super) const MONITOR_COLOR: [f32; 4] = [0.75, 0.73, 0.68, 1.0];
/// Walnut-veneer consumer TV, period-typical.
pub(super) const TV_COLOR: [f32; 4] = [0.30, 0.20, 0.13, 1.0];
pub(super) const CART_COLOR: [f32; 4] = [0.25, 0.25, 0.27, 1.0];
pub(super) const SWITCH_COLOR: [f32; 4] = [0.35, 0.35, 0.36, 1.0];
/// Room background the viewport clears to.
pub(super) const CLEAR_COLOR: [f32; 4] = [0.10, 0.11, 0.13, 1.0];
/// How far a hovered interactive prop's color is pushed toward white.
pub(super) const HIGHLIGHT_MIX: f32 = 0.25;

/// What a prop *is* — placement, pickability, and machine-state coupling
/// key off this.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PropKind {
    Desk,
    /// The CM-8 analog-RGB monitor shell (drawn when the machine's monitor
    /// is RGB; clicking it swaps to the TV — i.e. re-plugs the cable).
    MonitorCm8,
    /// The consumer-TV shell (drawn when the monitor is composite).
    MonitorTv,
    Case,
    PowerSwitch,
    Cartridge,
    /// The CRT face: textured with the live framebuffer and drawn unlit
    /// (emissive), like a powered tube.
    Screen,
}

/// Pose of the cartridge on the desk (not inserted).
pub(super) fn cart_desk_pose() -> (Vec3, f32) {
    (Vec3::from(CART_POS), CART_YAW)
}

/// Pose of the cartridge seated in the slot.
pub(super) fn cart_slot_pose() -> (Vec3, f32) {
    (Vec3::from(CART_SLOT_POS), CART_SLOT_YAW)
}

/// Interpolated pak pose at eased progress `t` (0 = desk, 1 = seated),
/// travelling through a little arc.
pub(super) fn cart_anim_pose(t: f32) -> Mat4 {
    let eased = t * t * (3.0 - 2.0 * t);
    let (from_pos, from_yaw) = cart_desk_pose();
    let (to_pos, to_yaw) = cart_slot_pose();
    let mut pos = from_pos.lerp(to_pos, eased);
    pos.y += (eased * std::f32::consts::PI).sin() * CART_ANIM_ARC;
    let yaw = from_yaw + (to_yaw - from_yaw) * eased;
    Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw)
}

/// Where the additive bezel-glow quad sits: over the screen quad, scaled up
/// to spill onto the monitor shell.
pub(super) fn glow_quad_transform() -> Mat4 {
    Mat4::from_translation(Vec3::from(SCREEN_POS) + Vec3::new(0.0, 0.0, GLOW_QUAD_LIFT))
        * Mat4::from_scale(Vec3::new(GLOW_QUAD_SCALE, GLOW_QUAD_SCALE, 1.0))
}

// ---------------------------------------------------------------------------
// Orbit camera.
// ---------------------------------------------------------------------------

/// Point the camera orbits: roughly the top of the case / bottom of the
/// screen, so both stay framed.
const ORBIT_TARGET: [f32; 3] = [0.0, 0.25, 0.0];
pub(super) const ORBIT_RADIANS_PER_POINT: f32 = 0.01;
/// Exponential zoom factor per scroll point (trackpad-friendly).
pub(super) const ZOOM_PER_SCROLL_POINT: f32 = 0.002;
pub(super) const PITCH_MIN: f32 = -0.05;
pub(super) const PITCH_MAX: f32 = 1.35;
pub(super) const DIST_MIN: f32 = 0.4;
pub(super) const DIST_MAX: f32 = 4.0;
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_4;
const Z_NEAR: f32 = 0.05;
const Z_FAR: f32 = 20.0;

pub(super) struct OrbitCamera {
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) dist: f32,
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
    pub(super) fn view_proj(&self, aspect: f32) -> Mat4 {
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
