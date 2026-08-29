use coco_core::keyboard::{self as kbd, Pos};
use eframe::egui;

/// Positional map: host physical key → CoCo matrix position (MAME's layout).
/// CLEAR also answers to backquote — added because laptop keyboards often
/// lack a Home key.
pub(crate) fn key_to_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        // Letters: @ A..Z run linearly from (0,0).
        K::A => (0, 1),
        K::B => (0, 2),
        K::C => (0, 3),
        K::D => (0, 4),
        K::E => (0, 5),
        K::F => (0, 6),
        K::G => (0, 7),
        K::H => (1, 0),
        K::I => (1, 1),
        K::J => (1, 2),
        K::K => (1, 3),
        K::L => (1, 4),
        K::M => (1, 5),
        K::N => (1, 6),
        K::O => (1, 7),
        K::P => (2, 0),
        K::Q => (2, 1),
        K::R => (2, 2),
        K::S => (2, 3),
        K::T => (2, 4),
        K::U => (2, 5),
        K::V => (2, 6),
        K::W => (2, 7),
        K::X => (3, 0),
        K::Y => (3, 1),
        K::Z => (3, 2),
        // Digits.
        K::Num0 => (4, 0),
        K::Num1 => (4, 1),
        K::Num2 => (4, 2),
        K::Num3 => (4, 3),
        K::Num4 => (4, 4),
        K::Num5 => (4, 5),
        K::Num6 => (4, 6),
        K::Num7 => (4, 7),
        K::Num8 => (5, 0),
        K::Num9 => (5, 1),
        // Punctuation (host physical key → CoCo key at that position, per MAME).
        K::Minus => (5, 2),     // CoCo ':'
        K::Semicolon => (5, 3), // CoCo ';'
        K::Comma => (5, 4),     // CoCo ','
        K::Equals => (5, 5),    // CoCo '-'
        K::Period => (5, 6),    // CoCo '.'
        K::Slash => (5, 7),     // CoCo '/'
        K::OpenBracket => kbd::AT,
        // Movement / control.
        K::Space => kbd::SPACE,
        K::Enter => kbd::ENTER,
        K::Backspace => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowLeft => kbd::LEFT,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home | K::Backtick => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
}

/// Control keys that symbolic mode still routes positionally (they produce no text).
pub(crate) fn control_key_pos(key: egui::Key) -> Option<Pos> {
    use egui::Key as K;
    let pos = match key {
        K::Enter => kbd::ENTER,
        K::Backspace | K::ArrowLeft => kbd::LEFT,
        K::ArrowUp => kbd::UP,
        K::ArrowDown => kbd::DOWN,
        K::ArrowRight => kbd::RIGHT,
        K::Escape => kbd::BREAK,
        K::Home | K::Backtick => kbd::CLEAR,
        K::F1 => kbd::F1,
        K::F2 => kbd::F2,
        _ => return None,
    };
    Some(pos)
}

/// Keys claimed by `joy::JoySource::Keys` (arrows, Z/X) once a port uses that
/// source — these stop reaching the CoCo keyboard matrix.
pub(crate) fn is_joystick_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::Z
            | egui::Key::X
    )
}

// `MachineVariant`/`MemorySize`/`VideoStandard` are all foreign types (defined
// in `coco-core`), so none of them can derive `clap::ValueEnum` here (orphan
// rule) without pulling a `clap` dependency into the core crate. Each gets a
// plain string `value_parser` function instead — same shape, no mirror enum.

#[cfg(test)]
#[path = "keymap_test.rs"]
mod tests;
