//! CRT shader stack (plan task 2). Six composable passes, each independently
//! toggleable from the CRT settings window; strength 0 / toggle off disables a
//! pass in the shader (uniform reaches it as 0).

/// Offscreen phosphor surface the framebuffer is accumulated onto (ping-pong
/// pair for the persistence blend). 4:3 like the tube; finer than any CoCo
/// mode so no detail is lost before the CRT-face pass resamples it.
pub(super) const PHOSPHOR_SIZE: [i32; 2] = [640, 480];
/// Small blurred copy of the phosphor surface driving bloom + bezel glow.
pub(super) const GLOW_SIZE: [i32; 2] = [160, 120];
/// The additive glow quad extends this far beyond the screen quad, spilling
/// tube light onto the bezel. Must match `GLOW_QUAD_SCALE` in the fragment
/// shader.
pub(super) const GLOW_QUAD_SCALE: f32 = 1.5;
/// Lift of the glow quad off the screen quad (which itself sits
/// [`SCREEN_LIFT`] off the monitor face).
pub(super) const GLOW_QUAD_LIFT: f32 = 0.004;
/// Phosphor decay 1.0 would never fade — cap the settings slider below it.
pub(super) const PERSISTENCE_MAX: f32 = 0.95;

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

// Deliberately subtle defaults (user feedback): the effect should read as
// "a real tube", not "a CRT filter" — every pass barely visible on its own,
// the sliders are there for anyone who wants more.
impl Default for CrtParams {
    fn default() -> Self {
        Self {
            barrel_on: true,
            barrel: 0.10,
            scanlines_on: true,
            scanlines: 0.12,
            mask_on: true,
            mask: 0.08,
            persistence_on: true,
            persistence: 0.30,
            bloom_on: true,
            bloom: 0.12,
            reflection_on: true,
            reflection: 0.08,
        }
    }
}

/// The uniform values one frame's callback needs, resolved from
/// [`CrtParams`] on the UI thread (toggle off → 0.0 → pass disabled).
#[derive(Clone, Copy)]
pub(super) struct CrtUniforms {
    /// barrel, scanlines, mask, bloom — the shader's `u_crt_a`.
    pub(super) a: [f32; 4],
    /// reflection, source scanline count, unused, unused — `u_crt_b`.
    pub(super) b: [f32; 4],
    /// Phosphor decay factor for the persistence pass.
    pub(super) decay: f32,
}

impl CrtParams {
    pub(super) fn uniforms(&self, fb_lines: f32) -> CrtUniforms {
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
