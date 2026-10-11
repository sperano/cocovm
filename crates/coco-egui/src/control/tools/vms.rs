//! The VM lifecycle tools: `list_vms`, `start_vm`, `stop_vm`, and
//! `suspend_vm`, plus how `list_vms` renders each [`VmInfo`].

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{Backend, RpcError, VmOnly, done, finish, parse_args, structured_result};
use crate::control::protocol::{Action, Reply, Request, VmInfo, VmMedia, model_id};
use crate::machine_def::{CartridgeDTO, SlotDTO};

/// Bytes per KiB, for `list_vms`'s `ram_kib`.
const BYTES_PER_KIB: usize = 1024;

pub(super) fn dispatch_list_vms(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    let _: VmOnly = parse_args(args)?;
    let req = Request {
        vm: None,
        action: Action::ListVms,
    };
    Ok(finish(backend, req, |reply| match reply {
        Reply::Vms(vms) => Some(structured_result(format_vms(&vms), vms_json(&vms))),
        _ => None,
    }))
}

/// Arguments of the lifecycle tools that act on one VM. `vm` is required:
/// these tools act on a whole machine, so they never guess the target.
#[derive(Deserialize)]
struct LifecycleArgs {
    vm: String,
}

fn dispatch_lifecycle(
    backend: &mut dyn Backend,
    args: Value,
    action: Action,
    message: &str,
) -> Result<Value, RpcError> {
    let LifecycleArgs { vm } = parse_args(args)?;
    let req = Request {
        vm: Some(vm),
        action,
    };
    Ok(finish(backend, req, |reply| done(reply, message)))
}

pub(super) fn dispatch_start_vm(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    dispatch_lifecycle(backend, args, Action::StartVm, "Started.")
}

pub(super) fn dispatch_stop_vm(backend: &mut dyn Backend, args: Value) -> Result<Value, RpcError> {
    dispatch_lifecycle(backend, args, Action::StopVm, "Stopped.")
}

pub(super) fn dispatch_suspend_vm(
    backend: &mut dyn Backend,
    args: Value,
) -> Result<Value, RpcError> {
    dispatch_lifecycle(backend, args, Action::SuspendVm, "Suspended.")
}

/// One block per VM: a heading line, then hardware, cartridge, and media.
fn format_vms(vms: &[VmInfo]) -> String {
    vms.iter().map(format_vm).collect::<Vec<_>>().join("\n")
}

fn format_vm(vm: &VmInfo) -> String {
    let media = match &vm.media {
        Some(media) => media_text(media),
        None => "unknown until resumed (kept in the suspended state)".to_string(),
    };
    format!(
        "{} — {} ({})\n  {}, {} RAM, {}\n  cartridge: {}\n  media: {media}",
        vm.slug,
        vm.name,
        vm.status.as_str(),
        crate::machine_label(vm.model),
        crate::new_vm::ram_label(vm.ram),
        vm.cpu.as_str(),
        cartridge_text(&vm.cartridge),
    )
}

fn cartridge_text(cartridge: &CartridgeDTO) -> String {
    match cartridge {
        CartridgeDTO::None => "none".to_string(),
        CartridgeDTO::FD502 { dos_rom } => format!("FD-502 ({})", dos_rom.label()),
        CartridgeDTO::ROMPak { path } => format!("ROM Pak {path}"),
        CartridgeDTO::BankedROMPak { path } => format!("banked ROM Pak {path}"),
        CartridgeDTO::RTC => "Disto RTC".to_string(),
        CartridgeDTO::RS232 { .. } => "Deluxe RS-232 Pak".to_string(),
        CartridgeDTO::GamesMaster { path } => format!("Games Master Cartridge {path}"),
        CartridgeDTO::Orch90 => "Orchestra-90".to_string(),
        CartridgeDTO::SoundSpeech => "Sound/Speech Cartridge".to_string(),
        CartridgeDTO::CoCoMax => "CoCo Max Hi-Res Input Module".to_string(),
        CartridgeDTO::MPI { slots, switch } => {
            let slots: Vec<String> = slots
                .iter()
                .enumerate()
                .map(|(i, slot)| format!("{} {}", i + 1, slot_text(slot)))
                .collect();
            format!(
                "MultiPak, switch on slot {switch}; slots: {}",
                slots.join(", ")
            )
        }
    }
}

fn slot_text(slot: &SlotDTO) -> String {
    match slot {
        SlotDTO::Empty => "empty".to_string(),
        SlotDTO::FD502 { dos_rom } => format!("FD-502 ({})", dos_rom.label()),
        SlotDTO::ROMPak { path } => format!("ROM Pak {path}"),
        SlotDTO::BankedROMPak { path } => format!("banked ROM Pak {path}"),
        SlotDTO::RTC => "Disto RTC".to_string(),
        SlotDTO::RS232 { .. } => "Deluxe RS-232 Pak".to_string(),
        SlotDTO::GamesMaster { path } => format!("Games Master Cartridge {path}"),
        SlotDTO::Orch90 => "Orchestra-90".to_string(),
        SlotDTO::SoundSpeech => "Sound/Speech Cartridge".to_string(),
        SlotDTO::CoCoMax => "CoCo Max Hi-Res Input Module".to_string(),
    }
}

/// Each mounted image as `<drive> <path>` (`disk0`, `vhd1`, `dw3`, `tape`),
/// or `none`.
fn media_text(media: &VmMedia) -> String {
    let mut mounted: Vec<String> = Vec::new();
    mounted.extend(indexed_paths("disk", &media.disks));
    mounted.extend(indexed_paths("vhd", &media.vhds));
    mounted.extend(indexed_paths("dw", &media.drivewire));
    if let Some(tape) = &media.tape {
        mounted.push(format!("tape {}", tape.display()));
    }
    if mounted.is_empty() {
        "none".to_string()
    } else {
        mounted.join(", ")
    }
}

fn indexed_paths<'a>(
    prefix: &'a str,
    paths: &'a [Option<PathBuf>],
) -> impl Iterator<Item = String> + 'a {
    paths.iter().enumerate().filter_map(move |(i, path)| {
        path.as_ref()
            .map(|path| format!("{prefix}{i} {}", path.display()))
    })
}

fn vms_json(vms: &[VmInfo]) -> Value {
    let vms: Vec<Value> = vms.iter().map(vm_json).collect();
    json!({"vms": vms})
}

/// One `list_vms` entry; `media` is left out when unknown.
fn vm_json(vm: &VmInfo) -> Value {
    let mut value = json!({
        "slug": vm.slug,
        "name": vm.name,
        "status": vm.status.as_str(),
        "model": model_id(vm.model),
        "ram_kib": vm.ram.bytes() / BYTES_PER_KIB,
        "cpu": vm.cpu.as_str(),
        "cartridge": serde_json::to_value(&vm.cartridge)
            .expect("a CartridgeDTO always serializes"),
    });
    if let Some(media) = &vm.media {
        value["media"] = media_json(media);
    }
    value
}

fn media_json(media: &VmMedia) -> Value {
    let paths = |paths: &[Option<PathBuf>]| -> Vec<Option<String>> {
        paths
            .iter()
            .map(|path| path.as_ref().map(|p| p.display().to_string()))
            .collect()
    };
    json!({
        "disks": paths(&media.disks),
        "vhds": paths(&media.vhds),
        "drivewire": paths(&media.drivewire),
        "tape": media.tape.as_ref().map(|p| p.display().to_string()),
    })
}

#[cfg(test)]
#[path = "vms_test.rs"]
pub(in crate::control) mod tests;
