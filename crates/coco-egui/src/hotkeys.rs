//! Rebindable UI hotkeys: the key-layout window (F10), the keyboard-mode
//! toggle (F12), New machine (⌘N), the debugger (⌘D), and the quick-load
//! (⌘1 to ⌘5) and quick-save (⇧⌘1 to ⇧⌘5) chords of the five quick states
//! (`save_state/quick.rs`). Each one is a `hotkey_*` key in `config.toml`
//! (`config.rs`), edited in the Settings dialog
//! (`manager/settings/hotkeys.rs`), and pushed into every VM window each
//! frame (`manager/vm_windows.rs`), like `toolbar_icons_only`.

use std::fmt;

use eframe::egui;

use crate::save_state::{QUICK_SLOTS, state_name};

/// `Cmd` in `config.toml`: egui's [`egui::Modifiers::COMMAND`], ⌘ on macOS
/// and Ctrl on Windows/Linux.
const CMD_NAME: &str = "Cmd";
/// Alias of [`CMD_NAME`].
const COMMAND_NAME: &str = "Command";
/// `Ctrl` in `config.toml`: the Control key. On Windows/Linux that is the
/// same key as [`CMD_NAME`], so it parses to COMMAND there.
const CTRL_NAME: &str = "Ctrl";
const ALT_NAME: &str = "Alt";
/// Alias of [`ALT_NAME`] (the macOS key cap).
const OPTION_NAME: &str = "Option";
const SHIFT_NAME: &str = "Shift";
/// Joins modifier names and the key name in `config.toml` (`Cmd+Shift+K`).
const SEPARATOR: char = '+';

/// Whether the Control key is distinct from COMMAND on this platform.
const SEPARATE_CTRL: bool = cfg!(target_os = "macos");

/// `config.toml` key prefix shared by every hotkey (`hotkey_key_layout`).
const CONFIG_KEY_PREFIX: &str = "hotkey_";

/// Digit keys of the default quick-state chords, State 1 to State 5.
const STATE_KEYS: [egui::Key; QUICK_SLOTS] = [
    egui::Key::Num1,
    egui::Key::Num2,
    egui::Key::Num3,
    egui::Key::Num4,
    egui::Key::Num5,
];

/// One hotkey: a logical key plus the modifiers held with it. The
/// modifiers are normalized ([`normalize`]), so two `Hotkey`s are equal
/// exactly when they fire on the same key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct Hotkey {
    modifiers: egui::Modifiers,
    key: egui::Key,
}

/// Shortcuts the app binds outside this module, which no hotkey may take:
/// the manager list's select-all (`manager/list.rs`), and the clipboard
/// chords egui turns into copy/cut/paste events.
pub(crate) fn reserved() -> impl Iterator<Item = egui::KeyboardShortcut> {
    let clipboard = [egui::Key::C, egui::Key::X, egui::Key::V]
        .map(|key| egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key));
    std::iter::once(crate::manager::list::SELECT_ALL_SHORTCUT).chain(clipboard)
}

/// Folds a pressed key's modifiers into the form [`Hotkey`] stores: the
/// platform's primary modifier as `command`, `ctrl` only where Control is a
/// separate key (macOS), and no `mac_cmd`. egui reports Ctrl on
/// Windows/Linux as `ctrl` and `command` together, and ⌘ on macOS as
/// `mac_cmd` and `command` together.
fn normalize(mods: egui::Modifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: mods.alt,
        ctrl: mods.ctrl && (mods.mac_cmd || !mods.command),
        shift: mods.shift,
        mac_cmd: false,
        command: mods.command,
    }
}

/// A letter or digit key (`A` to `Z`, `0` to `9`): the keys whose
/// [`egui::Key::name`] is the single character on the key cap.
fn is_alphanumeric(key: egui::Key) -> bool {
    let name = key.name().as_bytes();
    name.len() == 1 && name[0].is_ascii_alphanumeric()
}

/// A digit key (`0` to `9`).
fn is_digit(key: egui::Key) -> bool {
    let name = key.name().as_bytes();
    name.len() == 1 && name[0].is_ascii_digit()
}

/// The key a press names, for matching and for the Settings capture. The
/// logical key, except when the layout turned a digit-row key into
/// punctuation: egui-winit reports Shift+1 on a US layout as `!` with `1`
/// as the physical key, and that press is still ⇧⌘1 (as is ⌘& on an AZERTY
/// layout, whose digit row is unshifted punctuation). Only digits are
/// substituted: a letter key's logical key is its own letter on every
/// layout, and on Dvorak or AZERTY a punctuation key sits where QWERTY has
/// a letter, so ⌘. must stay ⌘. rather than become ⌘E.
fn pressed_key(key: egui::Key, physical_key: Option<egui::Key>) -> egui::Key {
    match physical_key {
        Some(physical) if !is_alphanumeric(key) && is_digit(physical) => physical,
        _ => key,
    }
}

impl Hotkey {
    /// A built-in default. Not validated: callers pass known-good bindings.
    pub(crate) const fn new(modifiers: egui::Modifiers, key: egui::Key) -> Self {
        Self { modifiers, key }
    }

    /// The hotkey for a press of `key` (physically `physical_key`) with
    /// `mods`, if it is one a hotkey may use ([`Hotkey::validate`]). The
    /// Settings dialog's capture path; names the key like [`pressed_key`].
    pub(crate) fn from_press(
        key: egui::Key,
        physical_key: Option<egui::Key>,
        mods: egui::Modifiers,
    ) -> Result<Self, String> {
        let hotkey = Self {
            modifiers: normalize(mods),
            key: pressed_key(key, physical_key),
        };
        hotkey.validate()?;
        Ok(hotkey)
    }

    /// For [`egui::Context::format_shortcut`], which spells it the
    /// platform's way (⌘D on macOS, Ctrl+D elsewhere).
    pub(crate) fn shortcut(self) -> egui::KeyboardShortcut {
        egui::KeyboardShortcut::new(self.modifiers, self.key)
    }

    /// Whether a press of `key` (physically `physical_key`) with `mods` is
    /// this hotkey. Exact: Shift+F10 does not fire an F10 hotkey.
    pub(crate) fn matches(
        self,
        key: egui::Key,
        physical_key: Option<egui::Key>,
        mods: egui::Modifiers,
    ) -> bool {
        pressed_key(key, physical_key) == self.key && normalize(mods) == self.modifiers
    }

    /// Removes every press of this hotkey (key repeats included) from
    /// `input`, so none reaches a widget or the CoCo matrix. True when one
    /// was a fresh press rather than a repeat.
    pub(crate) fn consume(self, input: &mut egui::InputState) -> bool {
        let mut fresh = false;
        input.events.retain(|event| match event {
            egui::Event::Key {
                key,
                physical_key,
                modifiers,
                pressed: true,
                repeat,
            } if self.matches(*key, *physical_key, *modifiers) => {
                fresh |= !repeat;
                false
            }
            _ => true,
        });
        fresh
    }

    /// A hotkey must leave typing alone and must not shadow another
    /// binding: without Cmd or Ctrl it has to be a function key the CoCo
    /// keyboard doesn't use (`keymap::key_to_pos`), and it can't be one of
    /// the [`reserved`] shortcuts. Alt doesn't count: egui-winit still sends
    /// the typed text with Alt held, and [`Hotkey::consume`] only removes
    /// the key event.
    fn validate(self) -> Result<(), String> {
        let m = self.modifiers;
        let types_text = !(m.command || m.ctrl);
        if types_text && (!is_function_key(self.key) || crate::key_to_pos(self.key).is_some()) {
            return Err(format!(
                "{self} would type into the machine: use a function key from F3 up, \
                 or add {CMD_NAME} or {CTRL_NAME}"
            ));
        }
        if reserved()
            .any(|shortcut| Self::new(normalize(shortcut.modifiers), shortcut.logical_key) == self)
        {
            return Err(format!("{self} is already taken by a built-in shortcut"));
        }
        Ok(())
    }
}

/// F1 through F35.
fn is_function_key(key: egui::Key) -> bool {
    key.name()
        .strip_prefix('F')
        .is_some_and(|n| n.parse::<u8>().is_ok())
}

/// Case-insensitive [`egui::Key::name`] lookup, falling back to egui's own
/// aliases (`Esc`, `ArrowUp`, ...).
fn key_from_name(name: &str) -> Option<egui::Key> {
    egui::Key::ALL
        .iter()
        .copied()
        .find(|key| key.name().eq_ignore_ascii_case(name))
        .or_else(|| egui::Key::from_name(name))
}

/// The flag in `mods` that the modifier `name` names, if any.
fn modifier_flag<'m>(mods: &'m mut egui::Modifiers, name: &str) -> Option<&'m mut bool> {
    let is = |candidate: &str| name.eq_ignore_ascii_case(candidate);
    if is(CMD_NAME) || is(COMMAND_NAME) || (is(CTRL_NAME) && !SEPARATE_CTRL) {
        Some(&mut mods.command)
    } else if is(CTRL_NAME) {
        Some(&mut mods.ctrl)
    } else if is(ALT_NAME) || is(OPTION_NAME) {
        Some(&mut mods.alt)
    } else if is(SHIFT_NAME) {
        Some(&mut mods.shift)
    } else {
        None
    }
}

impl std::str::FromStr for Hotkey {
    type Err = String;

    /// `config.toml`'s spelling: modifier names then a key name, joined
    /// by `+` (`F10`, `Cmd+D`, `Shift+F5`). Names are case-insensitive.
    fn from_str(text: &str) -> Result<Self, String> {
        let mut parts: Vec<&str> = text.split(SEPARATOR).map(str::trim).collect();
        let key_name = parts.pop().unwrap_or_default();
        let key = key_from_name(key_name)
            .ok_or_else(|| format!("hotkey {text:?}: unknown key {key_name:?}"))?;
        let mut modifiers = egui::Modifiers::NONE;
        for name in parts {
            let flag = modifier_flag(&mut modifiers, name).ok_or_else(|| {
                format!(
                    "hotkey {text:?}: unknown modifier {name:?} \
                     (expected {CMD_NAME}, {CTRL_NAME}, {ALT_NAME}, or {SHIFT_NAME})"
                )
            })?;
            // A repeat counts once: a macOS `Ctrl+Cmd+K` still loads on
            // Windows/Linux, where both name the Control key.
            *flag = true;
        }
        let hotkey = Self { modifiers, key };
        hotkey
            .validate()
            .map_err(|e| format!("hotkey {text:?}: {e}"))?;
        Ok(hotkey)
    }
}

impl TryFrom<String> for Hotkey {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        text.parse()
    }
}

impl fmt::Display for Hotkey {
    /// The [`std::str::FromStr`] spelling, so it round-trips through
    /// `config.toml`. On Windows/Linux COMMAND is written `Ctrl`, the key
    /// it is there.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        let command_name = if SEPARATE_CTRL { CMD_NAME } else { CTRL_NAME };
        let names = [
            (m.ctrl, CTRL_NAME),
            (m.command, command_name),
            (m.alt, ALT_NAME),
            (m.shift, SHIFT_NAME),
        ];
        for (_, name) in names.iter().filter(|(held, _)| *held) {
            write!(f, "{name}{SEPARATOR}")?;
        }
        f.write_str(self.key.name())
    }
}

/// One rebindable action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyAction {
    /// Show/hide the key-layout window (`kbd_help.rs`).
    KeyLayout,
    /// Switch positional/symbolic keyboard mode (`app/input.rs`).
    KeyboardMode,
    /// The manager's New machine (toolbar New…). VM windows swallow it
    /// without acting: creating machines is the manager's job.
    NewMachine,
    /// Open/close the debugger (`debugger.rs`, `debug-ui` builds only).
    Debugger,
    /// Quick-load the quick state with this 0-based slot
    /// (`save_state/quick.rs`).
    LoadState(usize),
    /// Quick-save to the quick state with this 0-based slot.
    SaveState(usize),
}

impl HotkeyAction {
    /// The single actions, in Settings-dialog order; the quick-state
    /// chords follow them in [`Self::all`].
    const SINGLE: [Self; 4] = [
        Self::KeyLayout,
        Self::KeyboardMode,
        Self::NewMachine,
        Self::Debugger,
    ];

    /// Every action, in Settings-dialog and `config.toml` order: the
    /// single actions, then Load State 1 to 5, then Save State 1 to 5. For
    /// clash checks too: a build without the debugger still keeps its
    /// binding free, so the file it saves loads in a `debug-ui` build.
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        let loads = (0..QUICK_SLOTS).map(Self::LoadState);
        let saves = (0..QUICK_SLOTS).map(Self::SaveState);
        Self::SINGLE.into_iter().chain(loads).chain(saves)
    }

    /// Every action this build acts on, in Settings-dialog order. The
    /// debugger exists only with the `debug-ui` feature; its `config.toml`
    /// key still parses without it, so one file serves every build.
    pub(crate) fn active() -> impl Iterator<Item = Self> {
        Self::all().filter(|action| cfg!(feature = "debug-ui") || *action != Self::Debugger)
    }

    /// The Settings dialog's row label.
    pub(crate) fn label(self) -> String {
        match self {
            Self::KeyLayout => "Key layout window".to_string(),
            Self::KeyboardMode => "Keyboard mode".to_string(),
            Self::NewMachine => "New machine".to_string(),
            Self::Debugger => "Debugger".to_string(),
            Self::LoadState(slot) => format!("Load {}", state_name(slot)),
            Self::SaveState(slot) => format!("Save {}", state_name(slot)),
        }
    }

    /// This action's `config.toml` key: `hotkey_key_layout`,
    /// `hotkey_load_state_1` (1-based, like the state's name).
    pub(crate) fn config_key(self) -> String {
        let suffix = match self {
            Self::KeyLayout => "key_layout".to_string(),
            Self::KeyboardMode => "keyboard_mode".to_string(),
            Self::NewMachine => "new_machine".to_string(),
            Self::Debugger => "debugger".to_string(),
            Self::LoadState(slot) => format!("load_state_{}", slot + 1),
            Self::SaveState(slot) => format!("save_state_{}", slot + 1),
        };
        format!("{CONFIG_KEY_PREFIX}{suffix}")
    }

    /// One-line description for the `config.toml` template's comment.
    pub(crate) fn description(self) -> String {
        match self {
            Self::KeyLayout => "show/hide the key layout window".to_string(),
            Self::KeyboardMode => "switch positional/symbolic keyboard mode".to_string(),
            Self::NewMachine => "create a new machine (manager window)".to_string(),
            Self::Debugger => "open/close the debugger (debug-ui builds)".to_string(),
            Self::LoadState(slot) => format!("quick-load {}", state_name(slot)),
            Self::SaveState(slot) => format!("quick-save to {}", state_name(slot)),
        }
    }
}

/// The resolved binding of every [`HotkeyAction`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hotkeys {
    pub(crate) key_layout: Hotkey,
    pub(crate) keyboard_mode: Hotkey,
    pub(crate) new_machine: Hotkey,
    pub(crate) debugger: Hotkey,
    /// Quick-load chords, indexed by 0-based state slot.
    pub(crate) load_state: [Hotkey; QUICK_SLOTS],
    /// Quick-save chords, indexed by 0-based state slot.
    pub(crate) save_state: [Hotkey; QUICK_SLOTS],
}

/// [`STATE_KEYS`] each under `modifiers`, for the default state chords.
const fn state_hotkeys(modifiers: egui::Modifiers) -> [Hotkey; QUICK_SLOTS] {
    let mut hotkeys = [Hotkey::new(modifiers, STATE_KEYS[0]); QUICK_SLOTS];
    let mut slot = 0;
    while slot < QUICK_SLOTS {
        hotkeys[slot] = Hotkey::new(modifiers, STATE_KEYS[slot]);
        slot += 1;
    }
    hotkeys
}

/// The built-in bindings: F10, F12, ⌘N, ⌘D, ⌘1 to ⌘5 to load a state and
/// ⇧⌘1 to ⇧⌘5 to save one.
pub(crate) const DEFAULT_HOTKEYS: Hotkeys = Hotkeys {
    key_layout: Hotkey::new(egui::Modifiers::NONE, egui::Key::F10),
    keyboard_mode: Hotkey::new(egui::Modifiers::NONE, egui::Key::F12),
    new_machine: Hotkey::new(egui::Modifiers::COMMAND, egui::Key::N),
    debugger: Hotkey::new(egui::Modifiers::COMMAND, egui::Key::D),
    load_state: state_hotkeys(egui::Modifiers::COMMAND),
    save_state: state_hotkeys(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT)),
};

impl Default for Hotkeys {
    fn default() -> Self {
        DEFAULT_HOTKEYS
    }
}

impl Hotkeys {
    pub(crate) fn get(&self, action: HotkeyAction) -> Hotkey {
        *self.binding(action)
    }

    pub(crate) fn set(&mut self, action: HotkeyAction, hotkey: Hotkey) {
        *self.binding_mut(action) = hotkey;
    }

    fn binding(&self, action: HotkeyAction) -> &Hotkey {
        match action {
            HotkeyAction::KeyLayout => &self.key_layout,
            HotkeyAction::KeyboardMode => &self.keyboard_mode,
            HotkeyAction::NewMachine => &self.new_machine,
            HotkeyAction::Debugger => &self.debugger,
            HotkeyAction::LoadState(slot) => &self.load_state[slot],
            HotkeyAction::SaveState(slot) => &self.save_state[slot],
        }
    }

    fn binding_mut(&mut self, action: HotkeyAction) -> &mut Hotkey {
        match action {
            HotkeyAction::KeyLayout => &mut self.key_layout,
            HotkeyAction::KeyboardMode => &mut self.keyboard_mode,
            HotkeyAction::NewMachine => &mut self.new_machine,
            HotkeyAction::Debugger => &mut self.debugger,
            HotkeyAction::LoadState(slot) => &mut self.load_state[slot],
            HotkeyAction::SaveState(slot) => &mut self.save_state[slot],
        }
    }

    /// The action other than `action` already bound to `hotkey`.
    pub(crate) fn holder(&self, hotkey: Hotkey, action: HotkeyAction) -> Option<HotkeyAction> {
        HotkeyAction::all().find(|other| *other != action && self.get(*other) == hotkey)
    }

    /// `Err` naming the first two actions that share a binding.
    pub(crate) fn check_distinct(&self) -> Result<(), String> {
        for action in HotkeyAction::all() {
            let hotkey = self.get(action);
            if let Some(other) = self.holder(hotkey, action) {
                return Err(format!(
                    "{} and {} hotkeys are both {hotkey}",
                    action.label(),
                    other.label()
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "hotkeys_test.rs"]
mod tests;
