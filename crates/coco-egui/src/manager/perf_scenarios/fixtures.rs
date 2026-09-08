use super::super::{MachineEntry, ManagerApp, SUSPEND_STATE_FILE, THUMBNAIL_FILE};
use super::config::{Config, PREVIEW_COUNT, PRINTER_PAGE_COUNT};
use crate::display::{Display, TV};
use crate::machine_def::MachineDef;
use coco_core::{MachineConfig, MachineVariant, MemorySize, MonitorType};

#[path = "../../../../coco-core/examples/perf/workloads.rs"]
#[allow(dead_code)] // Shared with the core-only harness, which also constructs synthetic ROMs.
mod workloads;

const BOOT_FIELDS: usize = 120;
const LINES_PER_PAGE: usize = 66;
const PRINTER_LINE: &[u8] = b"PERFORMANCE BASELINE 0123456789\r";
const PRINTER_NEWLINE: u8 = b'\r';
const SNAPSHOT_FILE: &str = "performance.ccstate";

pub(super) fn prepare(app: &mut ManagerApp, config: &Config) -> Result<(), String> {
    if !app.entries.is_empty() {
        return Err("performance scenarios require an empty isolated machines directory".into());
    }
    if config.name == "manager-idle" {
        return Ok(());
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

fn fill_printer(handle: &mut coco_core::dmp105::DMP105Handle) {
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
) -> Result<bool, String> {
    match config.name.as_str() {
        "snapshot" => {
            let path = app
                .artifacts_root
                .as_ref()
                .ok_or("artifact root required")?
                .join(SNAPSHOT_FILE);
            let vm = app.entries[0].vm.as_mut().ok_or("snapshot VM missing")?;
            vm.save_state_to(&path)?;
            vm.load_state_from(&path)?;
        }
        "lifecycle" => {
            const LIFECYCLE_STEPS: u64 = 6;
            match operation % LIFECYCLE_STEPS {
                0 => app.suspend_vm(0),
                1 => app.close_vm_window(0),
                2 => app.resume_vm(0),
                3 => app.stop_vm(0),
                4 => app.start_vm(0),
                _ => {
                    app.close_vm_window(0);
                    app.start_vm(0);
                }
            }
            check_entry(app, 0)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
