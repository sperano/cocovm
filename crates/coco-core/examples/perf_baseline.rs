//! Opt-in wall-duration measurements; no machine-speed pass/fail thresholds.
#[path = "perf/allocations.rs"]
mod allocations;
#[path = "perf/workloads.rs"]
mod workloads;

use coco_core::{Machine, MachineConfig};
use serde_json::json;
use std::time::{Duration, Instant};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;
const DEFAULT_WARMUP_SECS: f64 = 3.0;
const DEFAULT_DURATION_SECS: f64 = 10.0;
const DEFAULT_REPEATS: usize = 3;
const BOOT_FIELDS: usize = 120;

fn seconds(name: &str, default: f64) -> Duration {
    let value =
        std::env::var(name).map_or(default, |v| v.parse().expect("seconds must be numeric"));
    assert!(
        value.is_finite() && value > 0.0,
        "seconds must be positive and finite"
    );
    Duration::from_secs_f64(value)
}

fn machine(scenario: &str) -> Machine {
    let rom = if scenario == "basic-idle" {
        std::fs::read(test_assets::rom(test_assets::rom::COCO3)).expect("install coco3.rom assets")
    } else {
        vec![0; workloads::ROM_BYTES]
    };
    let mut machine = Machine::new(MachineConfig::default(), rom.into_boxed_slice());
    match scenario {
        "basic-idle" => {
            for _ in 0..BOOT_FIELDS {
                machine.run_field();
                machine.take_audio().count();
            }
        }
        "dac" => workloads::configure_dac(&mut machine),
        "cartridge" => workloads::configure_cartridge(&mut machine),
        "graphics" => workloads::configure_graphics(&mut machine),
        _ => panic!("unknown scenario {scenario}"),
    }
    if matches!(scenario, "dac" | "cartridge") {
        workloads::assert_changing_audio(&mut machine);
    }
    machine
}

fn run_for(machine: &mut Machine, duration: Duration) -> (u64, Duration) {
    let start = Instant::now();
    let mut fields = 0;
    while start.elapsed() < duration {
        machine.run_field();
        std::hint::black_box(machine.take_audio().count());
        fields += 1;
    }
    (fields, start.elapsed())
}

fn marker(suffix: &str) {
    if let Some(output) = std::env::var_os("COCOVM_PERF_OUTPUT") {
        let mut path = output;
        path.push(suffix);
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        std::fs::write(path, json!({"unix_seconds": seconds}).to_string()).unwrap();
    }
}

fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "dac".into());
    let warmup = seconds("COCOVM_PERF_WARMUP_SECS", DEFAULT_WARMUP_SECS);
    let duration = seconds("COCOVM_PERF_DURATION_SECS", DEFAULT_DURATION_SECS);
    let repeats = std::env::var("COCOVM_PERF_REPEATS").map_or(DEFAULT_REPEATS, |v| {
        v.parse().expect("repeats must be integer")
    });
    assert!(repeats > 0);
    let instrumented = std::env::var("COCOVM_PERF_ALLOCATIONS").as_deref() != Ok("0");
    for repeat in 0..repeats {
        let mut machine = machine(&scenario);
        run_for(&mut machine, warmup);
        marker(".started");
        allocations::start(instrumented);
        let (fields, elapsed) = run_for(&mut machine, duration);
        let (allocations, bytes) = allocations::stop();
        marker(".finished");
        println!(
            "{}",
            json!({"scenario": scenario, "repeat": repeat,
            "kind": "headless-core", "elapsed_secs": elapsed.as_secs_f64(),
            "warmup_secs": warmup.as_secs_f64(), "fields": fields,
            "fields_per_sec": fields as f64 / elapsed.as_secs_f64(),
            "allocation_tracking": instrumented, "allocations": allocations,
            "allocated_bytes": bytes, "vm_count": 1, "variant": "coco3",
            "video_standard": "ntsc", "ram_bytes": machine.bus.ram.len(),
            "framebuffer": [machine.fb_width, machine.fb_height],
            "audio": "core stereo grid drained each field; no host device",
            "profile": if cfg!(debug_assertions) { "dev" } else { "release" }})
        );
    }
}
