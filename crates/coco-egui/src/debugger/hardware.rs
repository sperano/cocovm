//! Hardware-state panel: GIME `$FF90`-`$FF9F` decode, MMU task/bank map,
//! PIA0/PIA1 port state, and cartridge-port line states. Read-only (nothing
//! here is meaningfully "editable" — these are latched/derived hardware
//! states, not CPU-visible registers with obvious edit semantics) and, apart
//! from [`coco_core::Machine::video_mode_summary`] and
//! [`coco_core::SystemBus::peek`]-free field reads, touches only `pub` struct
//! fields already exposed by `coco-core` — no new side-effect-free read paths
//! were needed for this panel.

use coco_core::Machine;
use coco_core::gime::{self, init0, init1};
use coco_core::pia::{MC6821, cr};
use eframe::egui;

pub(super) fn hardware_ui(ui: &mut egui::Ui, machine: &Machine) {
    let g = &machine.bus.gime;
    ui.label(machine.video_mode_summary());
    ui.separator();

    ui.label("INIT0 ($FF90)");
    ui.horizontal(|ui| {
        for (label, bit) in [
            ("COCO", init0::COCO),
            ("MMUEN", init0::MMUEN),
            ("IEN", init0::IEN),
            ("FEN", init0::FEN),
            ("MC3", init0::MC3),
            ("MC2", init0::MC2),
            ("MC1", init0::MC1),
            ("MC0", init0::MC0),
        ] {
            ui.label(format!("{label}:{}", u8::from(g.init0 & bit != 0)));
        }
    });
    ui.label("INIT1 ($FF91)");
    ui.horizontal(|ui| {
        ui.label(format!("TINS:{}", u8::from(g.init1 & init1::TINS != 0)));
        ui.label(format!("TR:{}", u8::from(g.init1 & init1::TR != 0)));
    });
    ui.separator();

    ui.label(format!(
        "IRQ  enable:${:02X} pending:${:02X} (IEN={})",
        g.irq_enable,
        g.irq_pending,
        g.init0 & init0::IEN != 0
    ));
    ui.label(format!(
        "FIRQ enable:${:02X} pending:${:02X} (FEN={})",
        g.firq_enable,
        g.firq_pending,
        g.init0 & init0::FEN != 0
    ));
    ui.label(format!(
        "Timer reload:${:04X} count:${:04X} fast-clock:{}",
        g.timer_reload,
        g.timer_count,
        g.timer_is_fast()
    ));
    ui.separator();

    ui.label(format!(
        "MMU enabled:{} active task:{}",
        g.mmu_enabled, g.task
    ));
    for task in 0..gime::TASK_COUNT {
        let blocks: Vec<String> = g.mmu[task].iter().map(|b| format!("{b:02X}")).collect();
        ui.label(format!("  task {task}: {}", blocks.join(" ")));
    }
    ui.separator();

    pia_ui(ui, "PIA0", &machine.bus.pia0);
    pia_ui(ui, "PIA1", &machine.bus.pia1);
    ui.separator();

    ui.label(format!(
        "Cart lines: HALT*={} CART*-ties-Q={} NMI-pending={}",
        machine.bus.halt_asserted(),
        machine.bus.cart.cart_line_ties_q(),
        machine.bus.cart.nmi_pending(),
    ));
}

fn pia_ui(ui: &mut egui::Ui, name: &str, pia: &MC6821) {
    ui.label(name);
    egui::Grid::new(format!("dbg_{name}_grid")).show(ui, |ui| {
        ui.label("");
        ui.label("output");
        ui.label("ddr");
        ui.label("control");
        ui.label("input");
        ui.label("C1 flag");
        ui.end_row();
        for (label, port) in [("A", &pia.a), ("B", &pia.b)] {
            ui.label(label);
            ui.label(format!("${:02X}", port.output));
            ui.label(format!("${:02X}", port.ddr));
            ui.label(format!("${:02X}", port.control));
            ui.label(format!("${:02X}", port.input));
            ui.label(format!("{}", port.control & cr::C1_FLAG != 0));
            ui.end_row();
        }
    });
}
