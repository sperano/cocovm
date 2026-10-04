//! Per-VM handlers for the control protocol's mutating/read actions
//! (`crate::control::Action`). Dispatch, VM resolution, and pending-request
//! bookkeeping live in `manager::control` — this module only knows how to
//! apply one action to the `CocoApp` it's given.

use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use coco_core::keyboard::Pos;

use crate::typeahead::{FIELDS_PER_TAP, KeyTap};
use crate::*;

/// A `press_keys` hold in progress: the positions held down, released by
/// [`CocoApp::run_fields`] (`app/frame.rs`) once `fields_left` reaches 0.
pub(crate) struct RemoteHold {
    pub(crate) keys: Vec<Pos>,
    pub(crate) fields_left: u32,
}

/// A `joystick` request's override for one port, applied in
/// [`CocoApp::drive_joysticks`] (`app/input.rs`) after the host's own source.
pub(crate) struct RemoteStick {
    pub(crate) x: u8,
    pub(crate) y: u8,
    pub(crate) buttons: [bool; 2],
}

impl Default for RemoteStick {
    fn default() -> Self {
        Self {
            x: coco_core::joystick::AXIS_CENTER,
            y: coco_core::joystick::AXIS_CENTER,
            buttons: [false, false],
        }
    }
}

impl CocoApp {
    /// `screen_text`: decoded lines, video mode, and validated insertion point.
    pub(crate) fn screen_text(&mut self) -> crate::control::Reply {
        crate::control::Reply::Screen(self.screen_snapshot())
    }

    pub(crate) fn screen_snapshot(&mut self) -> crate::control::ScreenSnapshot {
        let screen = self.machine.text_screen();
        crate::control::ScreenSnapshot {
            lines: screen.lines,
            mode: self.machine.video_mode_summary(),
            cursor: screen.cursor,
        }
    }

    /// `screenshot`: the raw machine framebuffer (not the TV-processed
    /// display texture), PNG-encoded and base64'd.
    pub(crate) fn screenshot(&self) -> Result<crate::control::Reply, String> {
        let width = self.machine.fb_width;
        let height = self.machine.fb_height;
        let mut png_bytes = Vec::new();
        image::write_buffer_with_format(
            &mut Cursor::new(&mut png_bytes),
            &self.machine.framebuffer,
            width,
            height,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("could not encode screenshot: {e}"))?;
        Ok(crate::control::Reply::Screenshot {
            png_base64: BASE64.encode(png_bytes),
            width,
            height,
        })
    }

    /// `type_text`: queue `text` on [`Self::remote_type_ahead`], same
    /// mapping as [`Self::enqueue_text`] uses for the host's own type-ahead,
    /// except that a character with no CoCo key rejects the whole call
    /// (see [`crate::control::key_names::text_taps`]) instead of being
    /// dropped.
    /// Returns the fields the burst takes to drain into a target that scans
    /// the keyboard continuously; a busy target stretches each tap.
    pub(crate) fn start_remote_typing(&mut self, text: &str) -> Result<u64, String> {
        if !self.running {
            return Err("VM is paused; call set_running or start_vm first".to_string());
        }
        if self.remote_type_ahead.is_active() {
            return Err("a type_text burst is still draining".to_string());
        }
        let char_count = text.chars().count();
        if char_count > crate::control::MAX_TYPE_TEXT_CHARS {
            return Err(format!(
                "text is {char_count} characters; at most {} per call",
                crate::control::MAX_TYPE_TEXT_CHARS
            ));
        }
        let taps = crate::control::key_names::text_taps(text)?;
        let queued = taps.len() as u64;
        self.remote_type_ahead
            .queue
            .extend(taps.into_iter().map(KeyTap::from));
        Ok(queued * FIELDS_PER_TAP)
    }

    /// `press_keys`: hold every named key down together for `hold_fields`
    /// fields (clamped to [`crate::control::MAX_HOLD_FIELDS`]), then release.
    /// Returns the fields until release.
    pub(crate) fn start_remote_hold(
        &mut self,
        keys: &[String],
        hold_fields: Option<u32>,
    ) -> Result<u64, String> {
        if !self.running {
            return Err("VM is paused; call set_running or start_vm first".to_string());
        }
        if self.remote_held.is_some() {
            return Err("a press_keys hold is already in progress".to_string());
        }
        let mut positions = Vec::with_capacity(keys.len());
        let mut needs_shift = false;
        for name in keys {
            let Some((pos, shift)) = crate::control::key_names::key_pos(name) else {
                return Err(format!(
                    "unknown key {name:?}; accepted: {}",
                    crate::control::key_names::describe()
                ));
            };
            needs_shift |= shift;
            positions.push(pos);
        }
        if needs_shift {
            positions.push(kbd::SHIFT);
        }
        let fields_left = hold_fields
            .unwrap_or(crate::control::DEFAULT_HOLD_FIELDS)
            .min(crate::control::MAX_HOLD_FIELDS);
        for &pos in &positions {
            self.machine.bus.keyboard.set(pos, true);
        }
        self.remote_held = Some(RemoteHold {
            keys: positions,
            fields_left,
        });
        Ok(u64::from(fields_left))
    }

    /// `joystick`: set or release `stick`'s remote override.
    pub(crate) fn apply_remote_joystick(
        &mut self,
        stick: crate::control::Stick,
        x: Option<u8>,
        y: Option<u8>,
        button1: Option<bool>,
        button2: Option<bool>,
        release: bool,
    ) {
        let port = stick.index();
        if release {
            self.remote_joy[port] = None;
            return;
        }
        let mut state = self.remote_joy[port].take().unwrap_or_default();
        if let Some(x) = x {
            state.x = x.min(coco_core::joystick::AXIS_MAX);
        }
        if let Some(y) = y {
            state.y = y.min(coco_core::joystick::AXIS_MAX);
        }
        if let Some(pressed) = button1 {
            state.buttons[0] = pressed;
        }
        if let Some(pressed) = button2 {
            state.buttons[1] = pressed;
        }
        self.remote_joy[port] = Some(state);
    }

    /// `reset`: soft reset, or `hard` power-cycle (mirrors the toolbar's
    /// Reset tile, `chrome/toolbar.rs`).
    pub(crate) fn remote_reset(&mut self, hard: bool) {
        if hard {
            self.power_cycle();
        } else {
            self.machine.reset();
        }
    }

    /// `peek`: read `len` bytes from `addr` without bus side effects,
    /// clamped to [`crate::control::MAX_PEEK_LEN`].
    pub(crate) fn peek_bytes(&self, addr: u16, len: u16) -> Vec<u8> {
        let clamped = len.min(crate::control::MAX_PEEK_LEN);
        (0..clamped)
            .map(|i| self.machine.bus.peek(addr.wrapping_add(i)))
            .collect()
    }

    /// `poke`: write `bytes` starting at `addr`, with full bus side effects;
    /// at most [`crate::control::MAX_POKE_LEN`] bytes.
    pub(crate) fn poke_bytes(&mut self, addr: u16, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > crate::control::MAX_POKE_LEN {
            return Err(format!(
                "{} bytes; at most {} per poke",
                bytes.len(),
                crate::control::MAX_POKE_LEN
            ));
        }
        for (i, &b) in bytes.iter().enumerate() {
            self.machine.poke(addr.wrapping_add(i as u16), b);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "control_test.rs"]
mod tests;
