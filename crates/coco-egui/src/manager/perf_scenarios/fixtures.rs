use super::super::{MachineEntry, ManagerApp, SUSPEND_STATE_FILE, THUMBNAIL_FILE};
use super::config::{Config, PREVIEW_COUNT, PRINTER_PAGE_COUNT};
use crate::display::{Display, TV};
use crate::machine_def::MachineDef;
use coco_core::{MachineConfig, MachineVariant, MemorySize, MonitorType};
use std::time::{Duration, Instant};

#[path = "../../../../coco-core/examples/perf/workloads.rs"]
#[allow(dead_code)] // Shared with the core-only harness, which also constructs synthetic ROMs.
mod workloads;

const BOOT_FIELDS: usize = 120;
const LINES_PER_PAGE: usize = 66;
const PRINTER_LINE: &[u8] = b"PERFORMANCE BASELINE 0123456789\r";
const PRINTER_NEWLINE: u8 = b'\r';
const SNAPSHOT_FILE: &str = "performance.ccstate";
const LIFECYCLE_STEP_COUNT: u64 = 9;
const SCROLL_PHASE_COUNT: u64 = 3;

pub(super) const MANAGER_SCROLL_SURFACE: &str = "manager-list";
pub(super) const PRINTER_SCROLL_SURFACE: &str = "printer-paper";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScrollPosition {
    Beginning,
    Middle,
    End,
}

impl ScrollPosition {
    fn for_operation(operation: u64) -> Self {
        match operation % SCROLL_PHASE_COUNT {
            0 => Self::Beginning,
            1 => Self::Middle,
            _ => Self::End,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Beginning => "beginning",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }

    fn target_index(self, item_count: usize) -> usize {
        match self {
            Self::Beginning => 0,
            Self::Middle => (item_count / 2).min(item_count.saturating_sub(1)),
            Self::End => item_count.saturating_sub(1),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PerformedScroll {
    pub(super) surface: &'static str,
    pub(super) position: ScrollPosition,
    pub(super) target_index: usize,
}

pub(super) struct PerformedOperation {
    pub(super) name: &'static str,
    pub(super) cycle: Option<u64>,
    pub(super) cycle_step: Option<u64>,
    pub(super) scroll: Option<PerformedScroll>,
}

pub(super) struct OperationAttempt {
    pub(super) operation: PerformedOperation,
    pub(super) duration: Duration,
    pub(super) outcome: Result<(), String>,
}

pub(super) fn prepare(app: &mut ManagerApp, config: &Config) -> Result<(), String> {
    if !app.entries.is_empty() {
        return Err("performance scenarios require an empty isolated machines directory".into());
    }
    if config.name == "manager-idle" {
        return Ok(());
    }
    if config.name == "snapshot" {
        let root = app
            .artifacts_root
            .as_ref()
            .ok_or("artifact root required")?;
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    }
    let count = if config.name == "saved-previews" {
        1
    } else {
        config.vm_count
    };
    for index in 0..count {
        add_vm(app, config, index)?;
    }
    if config.name == "saved-previews" {
        return prepare_previews(app);
    }
    for index in 0..count {
        let vm = app.entries[index].vm.as_mut().expect("launched");
        match config.name.as_str() {
            "graphics" => workloads::configure_graphics(&mut vm.machine),
            "dac" => workloads::configure_dac(&mut vm.machine),
            "cartridge" => workloads::configure_cartridge(&mut vm.machine),
            "paused" => vm.set_running(false),
            "printer" => prepare_printer(vm),
            _ => {}
        }
        if ["dac", "cartridge"].contains(&config.name.as_str()) {
            workloads::assert_changing_audio(&mut vm.machine);
        }
        if config.name == "suspended" {
            app.suspend_vm(index);
            check_entry(app, index)?;
        }
    }
    Ok(())
}

fn add_vm(app: &mut ManagerApp, config: &Config, index: usize) -> Result<(), String> {
    let variant = if config.variant == "coco2" {
        MachineVariant::Coco2
    } else {
        MachineVariant::Coco3
    };
    let display = display(&config.display);
    let hardware = MachineConfig {
        variant,
        memory: if variant == MachineVariant::Coco2 {
            MemorySize::K64
        } else {
            MemorySize::K512
        },
        monitor: display.to_monitor(variant),
        vdg: crate::default_vdg(variant),
        ..MachineConfig::default()
    };
    hardware.validate()?;
    let slug = format!("perf-{index:04}");
    let mut def = MachineDef::from_config(format!("Performance {index}"), None, &hardware);
    def.hardware.display = Some(display.into());
    let dir = app
        .machines_dir
        .as_ref()
        .ok_or("isolated config directory is required")?;
    crate::machine_def::save(dir, &slug, &def)?;
    app.entries.push(MachineEntry::new(slug, def));
    app.start_vm(index);
    check_entry(app, index)?;
    let vm = app.entries[index]
        .vm
        .as_mut()
        .ok_or("VM launch produced no VM")?;
    for _ in 0..BOOT_FIELDS {
        vm.machine.run_field();
    }
    vm.machine.take_audio().for_each(drop);
    vm.reset_audio();
    Ok(())
}

fn display(name: &str) -> Display {
    match name {
        "cmp" => Display::Monitor(MonitorType::Composite),
        "tv" => Display::TV(TV::Color),
        "tv-bw" => Display::TV(TV::BW),
        _ => Display::Monitor(MonitorType::RGB),
    }
}

fn check_entry(app: &ManagerApp, index: usize) -> Result<(), String> {
    if let Some(error) = &app.entries[index].launch_error {
        return Err(error.clone());
    }
    if let Some(error) = &app.save_error {
        return Err(error.clone());
    }
    Ok(())
}

fn prepare_printer(vm: &mut crate::CocoApp) {
    vm.toggle_paper_window();
    let mut handle = vm
        .paper_window
        .handle
        .as_ref()
        .expect("printer attached")
        .clone();
    fill_printer(&mut handle);
}

fn fill_printer(handle: &mut coco_core::dmp::DmpHandle) {
    use coco_core::bitbanger::PrinterSink;
    for _ in 0..PRINTER_PAGE_COUNT {
        for &byte in PRINTER_LINE {
            handle.write_byte(byte);
        }
        for _ in 1..LINES_PER_PAGE {
            handle.write_byte(PRINTER_NEWLINE);
        }
    }
}

#[cfg(test)]
#[path = "fixtures_test.rs"]
mod tests;

fn prepare_previews(app: &mut ManagerApp) -> Result<(), String> {
    app.suspend_vm(0);
    check_entry(app, 0)?;
    app.close_vm_window(0);
    let root = app
        .artifacts_root
        .clone()
        .ok_or("isolated artifact directory is required")?;
    let config_dir = app
        .machines_dir
        .clone()
        .ok_or("isolated config directory is required")?;
    let source = root.join(&app.entries[0].slug);
    for index in 1..PREVIEW_COUNT {
        let slug = format!("perf-{index:04}");
        let mut def = app.entries[0].def.clone();
        def.name = format!("Performance {index}");
        crate::machine_def::save(&config_dir, &slug, &def)?;
        let destination = root.join(&slug);
        std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        for file in [SUSPEND_STATE_FILE, THUMBNAIL_FILE] {
            std::fs::hard_link(source.join(file), destination.join(file))
                .map_err(|e| e.to_string())?;
        }
        let mut entry = MachineEntry::new(slug, def);
        entry.suspended = true;
        app.entries.push(entry);
    }
    Ok(())
}

pub(super) fn operate(
    app: &mut ManagerApp,
    config: &Config,
    operation: u64,
) -> Option<OperationAttempt> {
    match config.name.as_str() {
        "snapshot" => Some(timed_operation(
            PerformedOperation {
                name: "snapshot-round-trip",
                cycle: None,
                cycle_step: None,
                scroll: None,
            },
            || {
                let _operation = crate::perf::span(crate::perf::Stage::HostOperation);
                snapshot_round_trip(app)
            },
        )),
        "lifecycle" => Some(timed_operation(lifecycle_operation(operation), || {
            let _operation = crate::perf::span(crate::perf::Stage::HostOperation);
            lifecycle_step(app, operation)
        })),
        "saved-previews" => Some(successful_operation(scroll_operation(
            MANAGER_SCROLL_SURFACE,
            app.entries.len(),
            operation,
        ))),
        "printer" => {
            let performed = scroll_operation(PRINTER_SCROLL_SURFACE, PRINTER_PAGE_COUNT, operation);
            let target_page = performed.scroll.expect("scroll operation").target_index as u32;
            Some(timed_operation(performed, || {
                let vm = app.entries[0].vm.as_mut().ok_or("printer VM missing")?;
                vm.paper_window.request_perf_scroll(operation, target_page);
                Ok(())
            }))
        }
        _ => None,
    }
}

fn timed_operation(
    operation: PerformedOperation,
    action: impl FnOnce() -> Result<(), String>,
) -> OperationAttempt {
    let started = Instant::now();
    let outcome = action();
    OperationAttempt {
        operation,
        duration: started.elapsed(),
        outcome,
    }
}

fn successful_operation(operation: PerformedOperation) -> OperationAttempt {
    timed_operation(operation, || Ok(()))
}

fn scroll_operation(
    surface: &'static str,
    item_count: usize,
    operation: u64,
) -> PerformedOperation {
    let position = ScrollPosition::for_operation(operation);
    PerformedOperation {
        name: "scroll",
        cycle: Some(operation / SCROLL_PHASE_COUNT),
        cycle_step: Some(operation % SCROLL_PHASE_COUNT),
        scroll: Some(PerformedScroll {
            surface,
            position,
            target_index: position.target_index(item_count),
        }),
    }
}

fn snapshot_round_trip(app: &mut ManagerApp) -> Result<(), String> {
    let path = app
        .artifacts_root
        .as_ref()
        .ok_or("artifact root required")?
        .join(SNAPSHOT_FILE);
    let vm = app.entries[0].vm.as_mut().ok_or("snapshot VM missing")?;
    {
        let _save = crate::perf::span(crate::perf::Stage::SnapshotSave);
        vm.save_state_to(&path)?;
    }
    let _restore = crate::perf::span(crate::perf::Stage::SnapshotRestore);
    vm.load_state_from(&path)
}

fn lifecycle_step(app: &mut ManagerApp, operation: u64) -> Result<(), String> {
    let cycle_step = operation % LIFECYCLE_STEP_COUNT;
    match cycle_step {
        0 => app.suspend_vm(0),
        1 => app.resume_vm(0),
        2 => app.suspend_vm(0),
        3 => app.close_vm_window(0),
        4 => app.resume_vm(0),
        5 => app.stop_vm(0),
        6 => app.start_vm(0),
        7 => app.close_vm_window(0),
        _ => app.start_vm(0),
    }
    check_entry(app, 0)?;
    Ok(())
}

fn lifecycle_operation(operation: u64) -> PerformedOperation {
    let cycle_step = operation % LIFECYCLE_STEP_COUNT;
    let name = match cycle_step {
        0 => "suspend-for-warm-resume",
        1 => "resume-live-vm",
        2 => "suspend-for-window-close",
        3 => "close-suspended-window",
        4 => "resume-cold-vm",
        5 => "stop-vm",
        6 => "start-vm",
        7 => "close-running-window",
        _ => "restart-after-window-close",
    };
    PerformedOperation {
        name,
        cycle: Some(operation / LIFECYCLE_STEP_COUNT),
        cycle_step: Some(cycle_step),
        scroll: None,
    }
}
