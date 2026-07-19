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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;
use glam::{Mat4, Vec3};

mod crt;
mod gl;
mod layout;
mod mesh;
mod shaders;

pub use crt::CrtParams;

use gl::{GlScene, GlState, Instance};
use layout::PropKind;
use mesh::PropDef;

#[cfg(test)]
pub(crate) use layout::{CART_SLOT_WORLD, SWITCH_WORLD};

/// Depth bits requested from eframe (`NativeOptions::depth_buffer`) — eframe
/// defaults to 0 and egui itself never depth-tests, but the desk scene does.
pub const DEPTH_BUFFER_BITS: u8 = 24;

/// Env var naming a PNG path: after [`SNAPSHOT_DELAY_FRAMES`] painted 3D
/// frames (long enough for BASIC to reach its prompt), the frame is read back
/// with `glReadPixels` and written there, then the hook disarms. Debug/verify
/// machinery in the spirit of the headless-typing tricks — it proves what the
/// GL scene actually renders without needing host screen-capture permissions.
const SNAPSHOT_ENV: &str = "COCO_VIEW3D_SNAPSHOT";
const SNAPSHOT_DELAY_FRAMES: u32 = 240;

/// The slice of machine state the desk mirrors, passed in by the app every
/// frame so menu-driven changes move the props too.
#[derive(Clone, Copy)]
pub struct MachineView {
    /// False = the tube is dark and emulation is paused (power switch off).
    pub powered: bool,
    /// A cartridge is in the (direct) port — the pak sits in the slot.
    pub cart_inserted: bool,
    /// False while an MPI owns the port; the desk pak is then inert.
    pub cart_interactive: bool,
}

/// What the user physically did this frame. The app applies these through
/// the same methods the menus use — the desk never touches the core itself.
pub enum DeskAction {
    TogglePower,
    /// The empty-slot pak was clicked: open the ROM picker and, on a chosen
    /// file, call [`View3d::begin_cart_insert`].
    ChooseCartridge,
    /// Insert animation finished with this file: attach the pak now.
    InsertCartridge(PathBuf),
    /// Eject animation finished: pull the pak now.
    EjectCartridge,
}

pub struct DeskResponse {
    pub rect: egui::Rect,
    pub action: Option<DeskAction>,
}

/// Cartridge slide animation. The core call fires when the slide completes
/// (`DeskAction::{Insert,Eject}Cartridge`), never mid-flight.
enum CartAnim {
    Idle,
    Inserting { path: PathBuf, t: f32 },
    Ejecting { t: f32 },
}

pub struct View3d {
    /// View → "3D Desk" toggle; off = the classic flat display.
    pub enabled: bool,
    /// View → "CRT Settings…" window visibility.
    pub show_settings: bool,
    /// Live-editable CRT pass toggles/strengths (the settings window).
    pub crt: CrtParams,
    camera: layout::OrbitCamera,
    cart_anim: CartAnim,
    /// CPU-side props, loaded (glTF or placeholder) on first use.
    props: Option<Arc<Vec<PropDef>>>,
    /// GPU-side scene, created inside the first paint callback and shared
    /// with the per-frame callback closures.
    gl_state: Arc<Mutex<GlState>>,
    /// One-shot [`SNAPSHOT_ENV`] readback target with its frame countdown;
    /// consumed by the callback that paints frame zero of the countdown.
    snapshot: Arc<Mutex<Option<(PathBuf, u32)>>>,
}

impl View3d {
    pub fn new() -> Self {
        Self {
            enabled: false,
            show_settings: false,
            crt: CrtParams::default(),
            camera: layout::OrbitCamera::default(),
            cart_anim: CartAnim::Idle,
            props: None,
            gl_state: Arc::new(Mutex::new(GlState::Uninit)),
            snapshot: Arc::new(Mutex::new(
                std::env::var_os(SNAPSHOT_ENV)
                    .map(|p| (PathBuf::from(p), SNAPSHOT_DELAY_FRAMES)),
            )),
        }
    }

    /// Start the insert slide for `path` (called by the app after its ROM
    /// picker; the actual insert happens when the slide lands).
    pub fn begin_cart_insert(&mut self, path: PathBuf) {
        self.cart_anim = CartAnim::Inserting { path, t: 0.0 };
    }

    /// Orbit/zoom the camera from this frame's drag/scroll input.
    fn handle_camera_input(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if response.dragged() {
            let delta = response.drag_delta();
            self.camera.yaw -= delta.x * layout::ORBIT_RADIANS_PER_POINT;
            self.camera.pitch = (self.camera.pitch + delta.y * layout::ORBIT_RADIANS_PER_POINT)
                .clamp(layout::PITCH_MIN, layout::PITCH_MAX);
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.dist = (self.camera.dist
                    * (-scroll * layout::ZOOM_PER_SCROLL_POINT).exp())
                .clamp(layout::DIST_MIN, layout::DIST_MAX);
            }
        }
    }

    /// Pose of the cartridge for this frame, from the *current* animation
    /// state (before it's advanced) — completion actions fire the frame
    /// after the pak visually lands, so there's no desk-pose flicker while
    /// the app's `cart_inserted` catches up.
    fn cart_pose(&self, view: MachineView) -> Mat4 {
        match &self.cart_anim {
            CartAnim::Inserting { t, .. } => layout::cart_anim_pose(t.min(1.0)),
            CartAnim::Ejecting { t } => layout::cart_anim_pose((1.0 - t).max(0.0)),
            CartAnim::Idle => {
                let (pos, yaw) = if view.cart_inserted {
                    layout::cart_slot_pose()
                } else {
                    layout::cart_desk_pose()
                };
                Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw)
            }
        }
    }

    /// Step the insert/eject slide by this frame's `dt`, returning the
    /// completion action (if any) once the slide lands.
    fn advance_cart_anim(&mut self, ui: &egui::Ui) -> Option<DeskAction> {
        if matches!(self.cart_anim, CartAnim::Idle) {
            return None;
        }
        const MAX_ANIM_STEP_SECS: f32 = 0.1;
        let dt = ui.input(|i| i.stable_dt).min(MAX_ANIM_STEP_SECS);
        ui.ctx().request_repaint();
        let mut action = None;
        self.cart_anim = match std::mem::replace(&mut self.cart_anim, CartAnim::Idle) {
            CartAnim::Inserting { path, t } if t >= 1.0 => {
                action = Some(DeskAction::InsertCartridge(path));
                CartAnim::Idle
            }
            CartAnim::Inserting { path, t } => CartAnim::Inserting {
                path,
                t: t + dt / layout::CART_ANIM_SECS,
            },
            CartAnim::Ejecting { t } if t >= 1.0 => {
                action = Some(DeskAction::EjectCartridge);
                CartAnim::Idle
            }
            CartAnim::Ejecting { t } => CartAnim::Ejecting {
                t: t + dt / layout::CART_ANIM_SECS,
            },
            CartAnim::Idle => CartAnim::Idle,
        };
        action
    }

    /// Cast the pointer ray at the interactive props (world-space ray
    /// transformed into each prop's object space, slab-tested against its
    /// mesh AABB); nearest hit wins.
    fn pick_hovered_prop(
        &self,
        response: &egui::Response,
        rect: egui::Rect,
        view_proj_mat: &Mat4,
        props: &[PropDef],
        view: MachineView,
        prop_transform: &impl Fn(PropKind) -> Mat4,
    ) -> Option<PropKind> {
        let pointer = response.hover_pos()?;
        let inv = view_proj_mat.inverse();
        let ndc_x = (pointer.x - rect.left()) / rect.width() * 2.0 - 1.0;
        let ndc_y = 1.0 - (pointer.y - rect.top()) / rect.height() * 2.0;
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, -1.0));
        let far = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        let dir = (far - near).normalize();
        let mut best = f32::MAX;
        let mut hovered = None;
        for prop in props {
            let interactive = match prop.kind {
                PropKind::PowerSwitch => true,
                PropKind::Cartridge => {
                    view.cart_interactive && matches!(self.cart_anim, CartAnim::Idle)
                }
                _ => false,
            };
            if !interactive {
                continue;
            }
            let inv_model = prop_transform(prop.kind).inverse();
            let local_origin = inv_model.transform_point3(near);
            let local_dir = inv_model.transform_vector3(dir);
            if let Some(t) = prop.aabb.hit(local_origin, local_dir)
                && t < best
            {
                best = t;
                hovered = Some(prop.kind);
            }
        }
        hovered
    }

    /// Apply a click on an already-hovered prop: toggles the power switch,
    /// starts an eject slide, or asks the app to open the ROM picker.
    fn handle_prop_click(&mut self, kind: PropKind, view: MachineView) -> Option<DeskAction> {
        match kind {
            PropKind::PowerSwitch => Some(DeskAction::TogglePower),
            PropKind::Cartridge if view.cart_inserted => {
                self.cart_anim = CartAnim::Ejecting { t: 0.0 };
                None
            }
            PropKind::Cartridge => Some(DeskAction::ChooseCartridge),
            _ => None,
        }
    }

    /// Fill the available panel rect with the desk scene, `fb_tex` being the
    /// per-frame `coco-fb` texture (uploaded by `step_emulation`) and
    /// `fb_lines` its height in emulated scanlines (drives the scanline
    /// pass). Dragging orbits, scrolling zooms; clicking interactive props
    /// (power switch, cartridge) yields a [`DeskAction`].
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        fb_tex: egui::TextureId,
        fb_lines: f32,
        view: MachineView,
    ) -> DeskResponse {
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        self.handle_camera_input(ui, &response);

        let props = Arc::clone(self.props.get_or_insert_with(|| Arc::new(mesh::build_props())));

        let cart_transform = self.cart_pose(view);
        let mut action = self.advance_cart_anim(ui);

        let prop_transform = |kind: PropKind| -> Mat4 {
            match kind {
                PropKind::Desk => {
                    Mat4::from_translation(Vec3::new(0.0, -layout::DESK_SIZE[1] / 2.0, 0.0))
                }
                PropKind::Monitor => Mat4::from_translation(Vec3::from(layout::MONITOR_POS)),
                PropKind::Case => Mat4::from_translation(Vec3::from(layout::CASE_POS)),
                PropKind::PowerSwitch => {
                    let pressed = if view.powered {
                        Vec3::ZERO
                    } else {
                        Vec3::new(0.0, layout::SWITCH_PRESSED_OFFSET, 0.0)
                    };
                    Mat4::from_translation(Vec3::from(layout::SWITCH_POS) + pressed)
                }
                PropKind::Cartridge => cart_transform,
                PropKind::Screen => Mat4::from_translation(Vec3::from(layout::SCREEN_POS)),
            }
        };

        let view_proj_mat = self.camera.view_proj(rect.width() / rect.height());
        let hovered =
            self.pick_hovered_prop(&response, rect, &view_proj_mat, &props, view, &prop_transform);

        if hovered.is_some() {
            ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        }
        if action.is_none()
            && response.clicked()
            && let Some(kind) = hovered
        {
            action = self.handle_prop_click(kind, view);
        }

        let instances = build_instances(&props, &prop_transform, hovered);

        let view_proj = view_proj_mat.to_cols_array();
        let gl_state = Arc::clone(&self.gl_state);
        let snapshot = Arc::clone(&self.snapshot);
        let crt = self.crt.uniforms(fb_lines);
        // Powered off = dark tube: skip the framebuffer (and with it the
        // whole CRT pass chain) so the screen renders as its solid black.
        let fb_tex = view.powered.then_some(fb_tex);
        let callback = make_paint_callback(gl_state, snapshot, props, instances, view_proj, crt, fb_tex);
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(callback),
        });
        DeskResponse { rect, action }
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
                            crt::PERSISTENCE_MAX,
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

    /// Test-only: where a world-space point lands on screen for THIS view's
    /// camera framing `rect` — `ui_tests` aims synthetic pointer events with
    /// it, exercising the same projection the picking path inverts.
    #[cfg(test)]
    pub(crate) fn screen_pos_of(&self, world: [f32; 3], rect: egui::Rect) -> egui::Pos2 {
        let ndc = self
            .camera
            .view_proj(rect.width() / rect.height())
            .project_point3(Vec3::from(world));
        egui::pos2(
            rect.left() + (ndc.x + 1.0) / 2.0 * rect.width(),
            rect.top() + (1.0 - ndc.y) / 2.0 * rect.height(),
        )
    }

    /// Free the GL resources (called from `CocoApp::on_exit`, the only hook
    /// eframe hands the `glow::Context` back to).
    pub fn destroy(&self, gl: Option<&eframe::glow::Context>) {
        let Some(gl) = gl else { return };
        let mut state = self.gl_state.lock().unwrap();
        if let GlState::Ready(scene) = &*state {
            scene.destroy(gl);
        }
        *state = GlState::Uninit;
    }
}

/// One [`Instance`] per prop for this frame: current pose from
/// `prop_transform`, hover highlight from `hovered`.
fn build_instances(
    props: &[PropDef],
    prop_transform: &impl Fn(PropKind) -> Mat4,
    hovered: Option<PropKind>,
) -> Vec<Instance> {
    props
        .iter()
        .enumerate()
        .map(|(i, prop)| Instance {
            mesh_idx: i,
            transform: prop_transform(prop.kind).to_cols_array(),
            color: prop.color,
            is_screen: prop.kind == PropKind::Screen,
            highlight: if hovered == Some(prop.kind) {
                layout::HIGHLIGHT_MIX
            } else {
                0.0
            },
        })
        .collect()
}

/// Build this frame's paint callback: lazily creates the GL scene on first
/// use, then draws the prop instances and (if armed) services the one-shot
/// snapshot readback.
#[allow(clippy::too_many_arguments)]
fn make_paint_callback(
    gl_state: Arc<Mutex<GlState>>,
    snapshot: Arc<Mutex<Option<(PathBuf, u32)>>>,
    props: Arc<Vec<PropDef>>,
    instances: Vec<Instance>,
    view_proj: [f32; 16],
    crt: crt::CrtUniforms,
    fb_tex: Option<egui::TextureId>,
) -> eframe::egui_glow::CallbackFn {
    eframe::egui_glow::CallbackFn::new(move |info, painter| {
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
                &instances,
                &view_proj,
                fb_tex.and_then(|id| painter.texture(id)),
                crt,
                painter.intermediate_fbo(),
                [vp.left_px, vp.from_bottom_px, vp.width_px, vp.height_px],
            );
            let mut snapshot = snapshot.lock().unwrap();
            match snapshot.as_mut() {
                Some((path, 0)) => {
                    gl::save_snapshot(gl, &info, path);
                    *snapshot = None;
                }
                Some((_, frames_left)) => *frames_left -= 1,
                None => {}
            }
        }
    })
}
