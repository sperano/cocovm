//! Joystick input sources: drive the CoCo's two analog ports
//! (`coco_core::joystick`) from the mouse, a gamepad (using `gilrs`), or the
//! keyboard, selectable per port from the joysticks menu that the status bar's
//! joysticks entry pops up ([`JoystickInputs::menu_ui`]).

use coco_core::Machine;
use coco_core::joystick::{AXIS_CENTER, AXIS_MAX, AXIS_X, AXIS_Y, LEFT, RIGHT};
use eframe::egui;

use crate::machine_def::JoySourceDTO;

mod gamepad;
use gamepad::GamepadState;
pub(crate) use gamepad::SharedGamepad;

/// Pot floor (0 = fully left/up), named to match `joystick::AXIS_MAX` (63 =
/// fully right/down) rather than leaving a bare `0` at each call site.
const AXIS_MIN: u8 = 0;

/// Minimum absolute gamepad axis deflection (gilrs' -1.0..=1.0 range) that
/// counts as "in use" for the status bar's joystick activity light — small
/// enough to catch a deliberate push, large enough that stick drift/noise
/// near center doesn't light it permanently.
const PAD_DEFLECT: f32 = 0.2;

/// Where a joystick port's axes and buttons are read from.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum JoySource {
    #[default]
    None,
    Mouse,
    Gamepad,
    Keys,
}

impl JoySource {
    pub const ALL: [Self; 4] = [Self::None, Self::Mouse, Self::Gamepad, Self::Keys];

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Mouse => "Mouse",
            Self::Gamepad => "Gamepad",
            Self::Keys => "Keys",
        }
    }
}

// `[ui].joy_left`/`joy_right`'s `From` impls, kept here (rather than beside
// `machine_def::JoySourceDTO` itself) so both `manager::detail_map` (form ⇄
// definition) and `launch.rs` (definition ⇒ runtime) share one conversion
// instead of each hand-rolling their own match, the way `KbModeDTO` still
// does at each call site.
impl From<JoySourceDTO> for JoySource {
    fn from(dto: JoySourceDTO) -> Self {
        match dto {
            JoySourceDTO::None => Self::None,
            JoySourceDTO::Mouse => Self::Mouse,
            JoySourceDTO::Gamepad => Self::Gamepad,
            JoySourceDTO::Keys => Self::Keys,
        }
    }
}

impl From<JoySource> for JoySourceDTO {
    fn from(source: JoySource) -> Self {
        match source {
            JoySource::None => Self::None,
            JoySource::Mouse => Self::Mouse,
            JoySource::Gamepad => Self::Gamepad,
            JoySource::Keys => Self::Keys,
        }
    }
}

/// Host-side keys driving `JoySource::Keys`: arrows for the axes, Z/X for the
/// two fire buttons (mirroring the analog stick's button 0/1).
#[derive(Clone, Copy, Default)]
struct KeyState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    button0: bool,
    button1: bool,
}

/// Per-VM joystick selections and mouse state, with a handle to the one
/// application-lifetime gamepad backend.
pub struct JoystickInputs {
    /// Indexed by `coco_core::joystick::{RIGHT, LEFT}`.
    pub sources: [JoySource; 2],
    gamepad: SharedGamepad,
    /// Whether each port's source is currently being actively driven —
    /// set every frame in [`Self::apply`], read by the status bar's
    /// joystick activity light. Indexed like `sources`. What "actively
    /// driven" means depends on the source: a mouse port only lights on a
    /// button (pointer motion alone would light it constantly while the
    /// pointer merely sits over the display); a gamepad port lights on any
    /// held button or an axis deflected past [`PAD_DEFLECT`]; a keyboard
    /// port lights on any of its six mapped keys; `None` never lights.
    ///
    /// For the mouse source specifically, [`mouse_in_use`] receives the
    /// same gated button states [`Self::apply`] pushes to the machine —
    /// the [`Self::mouse_fire`] latches — so the indicator lights exactly
    /// when the emulated fire button is held, never on clicks landing on
    /// the chrome around the display.
    pub in_use: [bool; 2],
    /// Per-host-button latches (0 = primary, 1 = secondary): whether that
    /// button is down *and* its press began on the CoCo display (see
    /// [`press_began_on_display`]). Maintained by [`Self::update_mouse_fire`]
    /// from each button's own press/release events; this IS the emulated
    /// fire-button state for any `Mouse`-sourced port.
    mouse_fire: [bool; 2],
}

impl JoystickInputs {
    pub fn new(gamepad: SharedGamepad) -> Self {
        Self {
            // Off by default: a mouse silently driving the pots surprised more than it helped.
            sources: [JoySource::None, JoySource::None],
            gamepad,
            in_use: [false, false],
            mouse_fire: [false, false],
        }
    }

    pub fn gamepad_available(&self) -> bool {
        self.gamepad.available()
    }

    #[cfg(all(test, feature = "perf"))]
    pub(crate) fn shares_gamepad(&self, gamepad: &SharedGamepad) -> bool {
        self.gamepad.shares_backend(gamepad)
    }

    /// True if either port is set to `Keys`, meaning arrow/Z/X keys are claimed by
    /// the joystick and must not also reach the CoCo keyboard matrix.
    pub fn keys_active(&self) -> bool {
        self.sources.contains(&JoySource::Keys)
    }

    /// Update the [`Self::mouse_fire`] latches from this frame's
    /// pointer-button events, keeping a button held only if its own press
    /// began on the CoCo display. Latched per-button rather than through egui's
    /// shared `PointerState::press_origin`, which any release clears for both.
    fn update_mouse_fire(
        &mut self,
        ctx: &egui::Context,
        display_rect: egui::Rect,
        display_layer: egui::LayerId,
        primary_down: bool,
        secondary_down: bool,
    ) {
        // Collected first: `ctx.layer_id_at` can't run inside `ctx.input`'s closure (both lock
        // the context).
        let button_events: Vec<(egui::Pos2, usize, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|ev| match ev {
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        ..
                    } => {
                        let slot = match button {
                            egui::PointerButton::Primary => 0,
                            egui::PointerButton::Secondary => 1,
                            _ => return None,
                        };
                        Some((*pos, slot, *pressed))
                    }
                    _ => None,
                })
                .collect()
        });
        for (pos, slot, pressed) in button_events {
            self.mouse_fire[slot] = pressed
                && press_began_on_display(pos, ctx.layer_id_at(pos), display_rect, display_layer);
        }
        // Clears a latch whose release event never arrived (focus loss, missed events).
        self.mouse_fire[0] &= primary_down;
        self.mouse_fire[1] &= secondary_down;
    }

    fn key_state(ctx: &egui::Context) -> KeyState {
        // A focused widget owns typed arrows/Z/X. An unfocused viewport cannot reliably
        // deliver releases, so it must not retain or apply key-driven joystick state.
        if ctx.wants_keyboard_input() || !crate::app::has_keyboard_focus(ctx) {
            return KeyState::default();
        }
        ctx.input(|i| KeyState {
            left: i.key_down(egui::Key::ArrowLeft),
            right: i.key_down(egui::Key::ArrowRight),
            up: i.key_down(egui::Key::ArrowUp),
            down: i.key_down(egui::Key::ArrowDown),
            button0: i.key_down(egui::Key::Z),
            button1: i.key_down(egui::Key::X),
        })
    }

    /// Poll gamepad events and push the current axis/button state of every
    /// non-`None` port onto the bus; call once per `update()`. `active_rect`
    /// is the sub-rect of `display_rect` the mouse axes normalize over — a
    /// pointer in the border still drives them, pinned to full deflection.
    pub fn apply(
        &mut self,
        ctx: &egui::Context,
        display_rect: egui::Rect,
        active_rect: egui::Rect,
        display_layer: egui::LayerId,
        machine: &mut Machine,
    ) {
        let GamepadState { axes, buttons } = self.gamepad.poll();

        let (pointer_pos, primary_down, secondary_down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
            )
        });
        self.update_mouse_fire(
            ctx,
            display_rect,
            display_layer,
            primary_down,
            secondary_down,
        );
        let [fire0, fire1] = self.mouse_fire;
        let keys = Self::key_state(ctx);

        for stick in [RIGHT, LEFT] {
            self.in_use[stick] = match self.sources[stick] {
                JoySource::None => {
                    // Recenter so a port doesn't stay wherever the previous source left it.
                    machine.bus.joysticks.set_axis(stick, AXIS_X, AXIS_CENTER);
                    machine.bus.joysticks.set_axis(stick, AXIS_Y, AXIS_CENTER);
                    machine.bus.joysticks.set_button(stick, 0, false);
                    machine.bus.joysticks.set_button(stick, 1, false);
                    false
                }
                JoySource::Mouse => {
                    // Axes update only while the pointer is over `display_rect`; otherwise the
                    // pot holds its last position.
                    if let Some(pos) = pointer_pos
                        && display_rect.contains(pos)
                        && let Some((px, py)) = pot_axes_from_pointer(pos, active_rect)
                    {
                        machine.bus.joysticks.set_axis(stick, AXIS_X, px);
                        machine.bus.joysticks.set_axis(stick, AXIS_Y, py);
                    }
                    machine.bus.joysticks.set_button(stick, 0, fire0);
                    machine.bus.joysticks.set_button(stick, 1, fire1);
                    // Buttons only — pointer motion alone would light this constantly.
                    mouse_in_use(fire0, fire1)
                }
                JoySource::Gamepad => {
                    let x = pot_from_bipolar(axes[0]);
                    let y = pot_from_bipolar(axes[1]);
                    machine.bus.joysticks.set_axis(stick, AXIS_X, x);
                    machine.bus.joysticks.set_axis(stick, AXIS_Y, y);
                    machine.bus.joysticks.set_button(stick, 0, buttons[0]);
                    machine.bus.joysticks.set_button(stick, 1, buttons[1]);
                    gamepad_in_use(buttons, axes)
                }
                JoySource::Keys => {
                    let x = axis_from_keys(keys.left, keys.right);
                    let y = axis_from_keys(keys.up, keys.down);
                    machine.bus.joysticks.set_axis(stick, AXIS_X, x);
                    machine.bus.joysticks.set_axis(stick, AXIS_Y, y);
                    machine.bus.joysticks.set_button(stick, 0, keys.button0);
                    machine.bus.joysticks.set_button(stick, 1, keys.button1);
                    keys_in_use(keys)
                }
            };
        }
    }

    /// "Joysticks" menu contents: selectable sources per port, plus gamepad
    /// status. Deliberately not a nested `ComboBox` — its popup fights the
    /// menu's dismiss-on-outside-click handling in egui.
    pub fn menu_ui(&mut self, ui: &mut egui::Ui) {
        for (stick, name) in [(RIGHT, "Right stick"), (LEFT, "Left stick")] {
            ui.label(egui::RichText::new(name).strong());
            for source in JoySource::ALL {
                let selected = self.sources[stick] == source;
                if ui.selectable_label(selected, source.label()).clicked() {
                    self.sources[stick] = source;
                }
            }
            ui.separator();
        }
        ui.label(if self.gamepad_available() {
            "Gamepad: connected"
        } else {
            "Gamepad: unavailable"
        });
    }
}

/// Full deflection while exactly one of a key pair is held, else centered —
/// covers "neither held" and "both held" (for example, opposing arrows) the same way.
fn axis_from_keys(negative: bool, positive: bool) -> u8 {
    match (negative, positive) {
        (true, false) => AXIS_MIN,
        (false, true) => AXIS_MAX,
        _ => AXIS_CENTER,
    }
}

/// Whether a mouse press at `origin` began on the CoCo display: geometrically
/// inside `display_rect`, and not under any egui layer floating above it
/// (`origin_layer` is the topmost interactable layer there, `None` over bare
/// panels or the manager's own display window).
fn press_began_on_display(
    origin: egui::Pos2,
    origin_layer: Option<egui::LayerId>,
    display_rect: egui::Rect,
    display_layer: egui::LayerId,
) -> bool {
    display_rect.contains(origin) && origin_layer.is_none_or(|layer| layer == display_layer)
}

/// `JoySource::Mouse`'s "in use" test: buttons only, using the already
/// display-gated fire-button states (not raw host button states).
fn mouse_in_use(fire0: bool, fire1: bool) -> bool {
    fire0 || fire1
}

/// `JoySource::Gamepad`'s "in use" test: either mapped button held, or
/// either axis deflected past [`PAD_DEFLECT`].
fn gamepad_in_use(buttons: [bool; 2], axes: [f32; 2]) -> bool {
    buttons[0] || buttons[1] || axes.iter().any(|v| v.abs() > PAD_DEFLECT)
}

/// `JoySource::Keys`'s "in use" test: any of the six mapped keys held.
/// Takes `keys` by value — `KeyState` is `Copy`.
fn keys_in_use(keys: KeyState) -> bool {
    keys.left || keys.right || keys.up || keys.down || keys.button0 || keys.button1
}

/// Map a 0.0..=1.0 fraction (for example, a pointer position within the display rect) to
/// a 0..=63 pot value.
fn pot_from_unit(frac: f32) -> u8 {
    (frac.clamp(0.0, 1.0) * AXIS_MAX as f32).round() as u8
}

/// Map a mouse pointer position to `(pot_x, pot_y)` normalized over
/// `active_rect`. `None` on a degenerate `active_rect` (zero-size, negative,
/// or NaN extents), which would otherwise corrupt the pot state with NaN.
fn pot_axes_from_pointer(pos: egui::Pos2, active_rect: egui::Rect) -> Option<(u8, u8)> {
    if !(active_rect.width() > 0.0 && active_rect.height() > 0.0) {
        return None;
    }
    let nx = (pos.x - active_rect.left()) / active_rect.width();
    let ny = (pos.y - active_rect.top()) / active_rect.height();
    Some((pot_from_unit(nx), pot_from_unit(ny)))
}

/// Map a gilrs-style -1.0..=1.0 analog axis to a 0..=63 pot value.
fn pot_from_bipolar(v: f32) -> u8 {
    pot_from_unit((v.clamp(-1.0, 1.0) + 1.0) / 2.0)
}

#[cfg(test)]
#[path = "joy_test.rs"]
mod tests;
