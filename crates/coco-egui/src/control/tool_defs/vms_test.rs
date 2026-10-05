use std::collections::BTreeSet;

use super::*;
use crate::machine_def::{CartridgeDTO, RS232EndpointDTO, SlotDTO};

/// The `kind` tag `value` serializes with.
fn kind(value: impl serde::Serialize) -> String {
    serde_json::to_value(value).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}

fn set(kinds: &[&str]) -> BTreeSet<String> {
    kinds.iter().map(|k| k.to_string()).collect()
}

const PATH: &str = "/roms/pak.rom";

fn every_slot() -> Vec<SlotDTO> {
    let path = PATH.to_string();
    vec![
        SlotDTO::Empty,
        SlotDTO::FD502 {
            dos_rom: DosRom::DiskBasic,
        },
        SlotDTO::ROMPak { path: path.clone() },
        SlotDTO::BankedROMPak { path: path.clone() },
        SlotDTO::RTC,
        SlotDTO::RS232 {
            endpoint: RS232EndpointDTO::Loopback,
        },
        SlotDTO::GamesMaster { path },
        SlotDTO::Orch90,
        SlotDTO::SoundSpeech,
        SlotDTO::CoCoMax,
    ]
}

fn every_cartridge() -> Vec<CartridgeDTO> {
    let path = PATH.to_string();
    vec![
        CartridgeDTO::None,
        CartridgeDTO::FD502 {
            dos_rom: DosRom::HdbDosDw3,
        },
        CartridgeDTO::ROMPak { path: path.clone() },
        CartridgeDTO::BankedROMPak { path: path.clone() },
        CartridgeDTO::RTC,
        CartridgeDTO::RS232 {
            endpoint: RS232EndpointDTO::PTY,
        },
        CartridgeDTO::GamesMaster { path },
        CartridgeDTO::Orch90,
        CartridgeDTO::SoundSpeech,
        CartridgeDTO::CoCoMax,
        CartridgeDTO::MPI {
            slots: Default::default(),
            switch: MPI_SLOT_COUNT,
        },
    ]
}

#[test]
fn cartridge_kinds_match_the_definition_tags() {
    let tags: BTreeSet<String> = every_cartridge().iter().map(kind).collect();
    assert_eq!(tags, set(CARTRIDGE_KINDS));
}

#[test]
fn slot_kinds_match_the_definition_tags() {
    let tags: BTreeSet<String> = every_slot().iter().map(kind).collect();
    assert_eq!(tags, set(SLOT_KINDS));
}

#[test]
fn rs232_endpoint_kinds_match_the_definition_tags() {
    let endpoints = [
        RS232EndpointDTO::Loopback,
        RS232EndpointDTO::TCP {
            listen: "127.0.0.1:0".to_string(),
        },
        RS232EndpointDTO::PTY,
    ];
    let tags: BTreeSet<String> = endpoints.iter().map(kind).collect();
    assert_eq!(tags, set(RS232_ENDPOINT_KINDS));
}

#[test]
fn vm_schema_enumerates_every_model_and_cpu() {
    let schema = vm_schema();
    let models: Vec<&str> = MachineVariant::ALL.iter().map(|&v| model_id(v)).collect();
    assert_eq!(schema["properties"]["model"]["enum"], json!(models));
    assert_eq!(schema["properties"]["cpu"]["enum"], json!(["MC6809"]));
}

#[test]
fn media_is_the_only_optional_vm_property() {
    let schema = vm_schema();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    let optional: Vec<&str> = schema["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .filter(|p| !required.contains(p))
        .collect();
    assert_eq!(optional, ["media"]);
}
