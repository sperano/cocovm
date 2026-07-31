//! Joystick input sources: drive the CoCo's two analog ports
//! (`coco_core::joystick`) from the mouse, a gamepad (via `gilrs`), or the
//! keyboard, per-port selectable from the "Joysticks" menu.

use coco_core::Machine;
use coco_core::joystick::{AXIS_CENTER, AXIS_MAX, AXIS_X, AXIS_Y, LEFT, RIGHT};
use eframe::egui;

use crate::machine_def::JoySourceDTO;

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
#[derive(Clone, Copy)]
struct KeyState {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    button0: bool,
    button1: bool,
}

/// Runtime state for the two joystick ports: which source drives each, plus
/// the (optional) gilrs handle shared by any port set to `Gamepad`.
pub struct JoystickInputs {
    /// Indexed by `coco_core::joystick::{RIGHT, LEFT}`.
    pub sources: [JoySource; 2],
    /// `None` when no gamepad backend is available on this host (`gilrs::Gilrs::new`
    /// failed) — the `Gamepad` source is then selectable but stays centered.
    gilrs: Option<gilrs::Gilrs>,
    /// Left analog stick of the first gamepad seen, in gilrs' -1.0..=1.0 range.
    pad_axes: [f32; 2],
    /// South (button 0) / East (button 1) state of the first gamepad seen.
    pad_buttons: [bool; 2],
    /// Whether each port's source is currently being actively driven —
    /// set every frame in [`Self::apply`], read by the status bar's
    /// joystick activity light. Indexed like `sources`. What "actively
    /// driven" means depends on the source: a mouse port only lights on a
    /// button (pointer motion alone would light it constantly while the
    /// pointer merely sits over the display); a gamepad port lights on any
    /// held button or an axis deflected past [`PAD_DEFLECT`]; a keyboard
    /// port lights on any of its six mapped keys; `None` never lights.
    ///
    /// For the mouse source specifically, [`mouse_in_use`] lights on
    /// `primary_down`/`secondary_down` from anywhere in the egui input
    /// state — not gated by pointer position over the display. This
    /// matches [`Self::apply`]'s own button handling just above it (also
    /// ungated by `display_rect`): the emulated fire button tracks whatever
    /// the host mouse buttons are doing regardless of where the pointer
    /// sits, so a click anywhere in the window lights this indicator too,
    /// not only clicks over the CoCo display.
    pub in_use: [bool; 2],
}

impl JoystickInputs {
    pub fn new() -> Self {
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(e) => {
                tracing::warn!("gamepad input unavailable: {e}");
                None
            }
        };
        Self {
            // Both ports off until opted in (user decision 2026-07-29,
            // replacing the original right-stick-on-mouse default): a mouse
            // silently driving the pots surprised more than it helped, and
            // the Joysticks menu / manager form make enabling one a click.
            sources: [JoySource::None, JoySource::None],
            gilrs,
            pad_axes: [0.0, 0.0],
            pad_buttons: [false, false],
            in_use: [false, false],
        }
    }

    pub fn gamepad_available(&self) -> bool {
        self.gilrs.is_some()
    }

    /// True if either port is set to `Keys`, meaning arrow/Z/X keys are claimed by
    /// the joystick and must not also reach the CoCo keyboard matrix.
    pub fn keys_active(&self) -> bool {
        self.sources.contains(&JoySource::Keys)
    }

    /// Pump pending gilrs events, updating the shared `pad_axes`/`pad_buttons`
    /// from whichever gamepad reports them (effectively "first one to speak").
    /// D-pad presses are reported as `EventType::ButtonPressed` (gilrs' default
    /// filters turn the D-pad's axis into synthetic buttons) and are treated as
    /// full deflection, same as the requirement for keyboard arrows.
    fn poll_gamepad(&mut self) {
        use gilrs::{Axis, Button, EventType};

        let Some(gilrs) = self.gilrs.as_mut() else {
            return;
        };
        while let Some(gilrs::Event { event, .. }) = gilrs.next_event() {
            match event {
                EventType::AxisChanged(Axis::LeftStickX, v, _) => self.pad_axes[0] = v,
                // gilrs reports stick-up as a negative value (HID/SDL convention),
                // which already lines up with the CoCo pot's 0 = up.
                EventType::AxisChanged(Axis::LeftStickY, v, _) => self.pad_axes[1] = v,
                EventType::ButtonPressed(Button::DPadLeft, _) => self.pad_axes[0] = -1.0,
                EventType::ButtonPressed(Button::DPadRight, _) => self.pad_axes[0] = 1.0,
                EventType::ButtonReleased(Button::DPadLeft | Button::DPadRight, _) => {
                    self.pad_axes[0] = 0.0;
                }
                EventType::ButtonPressed(Button::DPadUp, _) => self.pad_axes[1] = -1.0,
                EventType::ButtonPressed(Button::DPadDown, _) => self.pad_axes[1] = 1.0,
                EventType::ButtonReleased(Button::DPadUp | Button::DPadDown, _) => {
                    self.pad_axes[1] = 0.0;
                }
                EventType::ButtonPressed(Button::South, _) => self.pad_buttons[0] = true,
                EventType::ButtonReleased(Button::South, _) => self.pad_buttons[0] = false,
                EventType::ButtonPressed(Button::East, _) => self.pad_buttons[1] = true,
                EventType::ButtonReleased(Button::East, _) => self.pad_buttons[1] = false,
                _ => {}
            }
        }
    }

    fn key_state(ctx: &egui::Context) -> KeyState {
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
    /// non-`None` port onto the bus. Call once per `update()`, before running
    /// any emulated fields.
    pub fn apply(&mut self, ctx: &egui::Context, display_rect: egui::Rect, machine: &mut Machine) {
        self.poll_gamepad();

        let (pointer_pos, primary_down, secondary_down) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
            )
        });
        let keys = Self::key_state(ctx);

        for stick in [RIGHT, LEFT] {
            self.in_use[stick] = match self.sources[stick] {
                JoySource::None => {
                    // Explicitly recenter so a port doesn't stay wherever a
                    // previously selected source last left it.
                    machine.bus.joysticks.set_axis(stick, AXIS_X, AXIS_CENTER);
                    machine.bus.joysticks.set_axis(stick, AXIS_Y, AXIS_CENTER);
                    machine.bus.joysticks.set_button(stick, 0, false);
                    machine.bus.joysticks.set_button(stick, 1, false);
                    false
                }
                JoySource::Mouse => {
                    // Only update the axes while the pointer is actually over the
                    // display; when it leaves, the pot holds its last position.
                    if let Some(pos) = pointer_pos
                        && display_rect.contains(pos)
                    {
                        let nx = (pos.x - display_rect.left()) / display_rect.width();
                        let ny = (pos.y - display_rect.top()) / display_rect.height();
                        machine
                            .bus
                            .joysticks
                            .set_axis(stick, AXIS_X, pot_from_unit(nx));
                        machine
                            .bus
                            .joysticks
                            .set_axis(stick, AXIS_Y, pot_from_unit(ny));
                    }
                    machine.bus.joysticks.set_button(stick, 0, primary_down);
                    machine.bus.joysticks.set_button(stick, 1, secondary_down);
                    // Buttons only: pointer motion alone would light this
                    // constantly whenever the pointer merely sits over the
                    // display.
                    mouse_in_use(primary_down, secondary_down)
                }
                JoySource::Gamepad => {
                    let x = pot_from_bipolar(self.pad_axes[0]);
                    let y = pot_from_bipolar(self.pad_axes[1]);
                    machine.bus.joysticks.set_axis(stick, AXIS_X, x);
                    machine.bus.joysticks.set_axis(stick, AXIS_Y, y);
                    machine
                        .bus
                        .joysticks
                        .set_button(stick, 0, self.pad_buttons[0]);
                    machine
                        .bus
                        .joysticks
                        .set_button(stick, 1, self.pad_buttons[1]);
                    gamepad_in_use(self.pad_buttons, self.pad_axes)
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

    /// "Joysticks" menu contents: a flat list of selectable sources per port, plus
    /// gamepad status. Deliberately not a nested `ComboBox` — a combo's own popup
    /// fights the menu's dismiss-on-outside-click handling in egui, so selections
    /// inside it don't register reliably.
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
/// covers "neither held" and "both held" (e.g. opposing arrows) the same way.
fn axis_from_keys(negative: bool, positive: bool) -> u8 {
    match (negative, positive) {
        (true, false) => AXIS_MIN,
        (false, true) => AXIS_MAX,
        _ => AXIS_CENTER,
    }
}

/// `JoySource::Mouse`'s "in use" test for the status bar's joystick
/// activity light: buttons only (see [`JoystickInputs::in_use`]'s doc
/// comment for why pointer position doesn't count).
fn mouse_in_use(primary_down: bool, secondary_down: bool) -> bool {
    primary_down || secondary_down
}

/// `JoySource::Gamepad`'s "in use" test: either mapped button held, or
/// either axis deflected past [`PAD_DEFLECT`].
fn gamepad_in_use(buttons: [bool; 2], axes: [f32; 2]) -> bool {
    buttons[0] || buttons[1] || axes.iter().any(|v| v.abs() > PAD_DEFLECT)
}

/// `JoySource::Keys`'s "in use" test: any of the six mapped keys held.
/// Takes `keys` by value, like [`gamepad_in_use`] takes its state — `KeyState`
/// is `Copy`, so there's no reason to borrow it.
fn keys_in_use(keys: KeyState) -> bool {
    keys.left || keys.right || keys.up || keys.down || keys.button0 || keys.button1
}

/// Map a 0.0..=1.0 fraction (e.g. pointer position within the display rect) to
/// a 0..=63 pot value.
fn pot_from_unit(frac: f32) -> u8 {
    (frac.clamp(0.0, 1.0) * AXIS_MAX as f32).round() as u8
}

/// Map a gilrs-style -1.0..=1.0 analog axis to a 0..=63 pot value.
fn pot_from_bipolar(v: f32) -> u8 {
    pot_from_unit((v.clamp(-1.0, 1.0) + 1.0) / 2.0)
}

#[cfg(test)]
#[path = "joy_test.rs"]
mod tests;
