//! On-screen keyboard-mapping overlay: draws the CoCo keyboard and, in positional
//! mode, the host key that drives each CoCo key. Toggled from the menu or F10.

use eframe::egui;

/// One key cap: (CoCo unshifted legend, CoCo shifted legend, host key in
/// positional mode). Empty strings are omitted.
type Cap = (&'static str, &'static str, &'static str);

const MAIN_ROWS: &[&[Cap]] = &[
    &[
        ("1", "!", "1"), ("2", "\"", "2"), ("3", "#", "3"), ("4", "$", "4"),
        ("5", "%", "5"), ("6", "&", "6"), ("7", "'", "7"), ("8", "(", "8"),
        ("9", ")", "9"), ("0", "", "0"), (":", "*", "-"), ("-", "=", "="),
    ],
    &[
        ("@", "", "["), ("Q", "", "Q"), ("W", "", "W"), ("E", "", "E"),
        ("R", "", "R"), ("T", "", "T"), ("Y", "", "Y"), ("U", "", "U"),
        ("I", "", "I"), ("O", "", "O"), ("P", "", "P"),
    ],
    &[
        ("A", "", "A"), ("S", "", "S"), ("D", "", "D"), ("F", "", "F"),
        ("G", "", "G"), ("H", "", "H"), ("J", "", "J"), ("K", "", "K"),
        ("L", "", "L"), (";", "+", ";"),
    ],
    &[
        ("Z", "", "Z"), ("X", "", "X"), ("C", "", "C"), ("V", "", "V"),
        ("B", "", "B"), ("N", "", "N"), ("M", "", "M"), (",", "<", ","),
        (".", ">", "."), ("/", "?", "/"),
    ],
];

const SPECIAL_ROW: &[Cap] = &[
    ("SHIFT", "", "⇧"),
    ("CTRL", "", "⌃"),
    ("ALT", "", "⌥"),
    ("SPACE", "", "Space"),
    ("ENTER", "", "Return"),
    ("CLEAR", "", "Home"),
    ("BREAK", "", "Esc"),
];

const ARROW_ROW: &[Cap] = &[
    ("↑", "", "↑"),
    ("↓", "", "↓"),
    ("←", "", "← / ⌫"),
    ("→", "", "→"),
    ("F1", "", "F1"),
    ("F2", "", "F2"),
];

const CAP_H: f32 = 44.0;
const HOST_GREEN: egui::Color32 = egui::Color32::from_rgb(0x30, 0xC0, 0x30);

fn keycap(ui: &mut egui::Ui, cap: Cap, width: f32, symbolic: bool) {
    let (main, shift, host) = cap;
    egui::Frame::group(ui.style())
        .inner_margin(3.0)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.set_height(CAP_H);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(main).strong());
                if !shift.is_empty() {
                    ui.label(egui::RichText::new(shift).small().weak());
                }
                if !symbolic {
                    ui.label(egui::RichText::new(host).small().color(HOST_GREEN));
                }
            });
        });
}

fn cap_row(ui: &mut egui::Ui, caps: &[Cap], width: f32, symbolic: bool) {
    ui.horizontal(|ui| {
        for &cap in caps {
            keycap(ui, cap, width, symbolic);
        }
    });
}

/// Draw the keyboard-mapping window. `open` is toggled by the window's close box.
pub fn window(ctx: &egui::Context, open: &mut bool, symbolic: bool) {
    const KEY_W: f32 = 42.0;
    const WIDE_W: f32 = 58.0;
    egui::Window::new(crate::window_title(ctx, "CoCo Keyboard Mapping"))
        .open(open)
        .resizable(false)
        .collapsible(true)
        .show(ctx, |ui| {
            if symbolic {
                ui.label("Symbolic mode — type the character you want; it is sent as typed.");
            } else {
                ui.label("Positional mode — each host key acts as the CoCo key in the same slot.");
                ui.label(egui::RichText::new("green = host key to press").small().color(HOST_GREEN));
            }
            ui.add_space(6.0);
            for row in MAIN_ROWS {
                cap_row(ui, row, KEY_W, symbolic);
            }
            ui.add_space(6.0);
            cap_row(ui, SPECIAL_ROW, WIDE_W, symbolic);
            cap_row(ui, ARROW_ROW, WIDE_W, symbolic);
            ui.add_space(4.0);
            ui.small("F12: toggle positional/symbolic   ·   F10: show/hide this help");
        });
}
