use std::time::Duration;

pub(super) const SCENARIOS: &[&str] = &[
    "manager-idle",
    "basic-idle",
    "graphics",
    "paused",
    "suspended",
    "background",
    "multi-vm",
    "tv",
    "dac",
    "cartridge",
    "saved-previews",
    "printer",
    "snapshot",
    "lifecycle",
    "control-load",
];
const DEFAULT_WARMUP_SECS: f64 = 3.0;
const DEFAULT_DURATION_SECS: f64 = 10.0;
const MAX_DURATION_SECS: f64 = 3600.0;
pub(super) const MANY_VM_COUNT: usize = 4;
pub(super) const PREVIEW_COUNT: usize = 500;
pub(super) const PRINTER_PAGE_COUNT: usize = 2000;

pub(super) struct Config {
    pub name: String,
    pub warmup: Duration,
    pub duration: Duration,
    pub output: std::path::PathBuf,
    pub vm_count: usize,
    pub display: String,
    pub variant: String,
}

impl Config {
    pub fn from_env() -> Result<Option<Self>, String> {
        let Ok(name) = std::env::var("COCOVM_PERF_SCENARIO") else {
            return Ok(None);
        };
        if !SCENARIOS.contains(&name.as_str()) {
            return Err(format!(
                "unknown performance scenario {name:?}; expected {SCENARIOS:?}"
            ));
        }
        let vm_count = vm_count(&name)?;
        let display = choice(
            "COCOVM_PERF_DISPLAY",
            if name == "tv" { "tv" } else { "rgb" },
            &["rgb", "cmp", "tv", "tv-bw"],
        )?;
        let variant = choice("COCOVM_PERF_VARIANT", "coco3", &["coco2", "coco3"])?;
        if name == "graphics" && variant != "coco3" {
            return Err("graphics workload requires coco3".into());
        }
        let output = std::env::var_os("COCOVM_PERF_OUTPUT")
            .ok_or("COCOVM_PERF_OUTPUT is required")?
            .into();
        Ok(Some(Self {
            name,
            warmup: duration("COCOVM_PERF_WARMUP_SECS", DEFAULT_WARMUP_SECS)?,
            duration: duration("COCOVM_PERF_DURATION_SECS", DEFAULT_DURATION_SECS)?,
            output,
            vm_count,
            display,
            variant,
        }))
    }
}

fn duration(name: &str, default: f64) -> Result<Duration, String> {
    let value = std::env::var(name)
        .ok()
        .map_or(Ok(default), |v| v.parse::<f64>().map_err(|e| e.to_string()))?;
    checked_duration(name, value)
}

fn checked_duration(name: &str, value: f64) -> Result<Duration, String> {
    if !value.is_finite() || value <= 0.0 || value > MAX_DURATION_SECS {
        return Err(format!(
            "{name} must be positive and at most {MAX_DURATION_SECS}"
        ));
    }
    Ok(Duration::from_secs_f64(value))
}

fn vm_count(name: &str) -> Result<usize, String> {
    let default = if name == "multi-vm" { MANY_VM_COUNT } else { 1 };
    let count = std::env::var("COCOVM_PERF_VM_COUNT")
        .ok()
        .map_or(Ok(default), |v| {
            v.parse::<usize>().map_err(|e| e.to_string())
        })?;
    if !(1..=MANY_VM_COUNT).contains(&count) {
        return Err("VM count must be 1 through 4".into());
    }
    Ok(count)
}

fn choice(name: &str, default: &str, allowed: &[&str]) -> Result<String, String> {
    let value = std::env::var(name).unwrap_or_else(|_| default.into());
    if !allowed.contains(&value.as_str()) {
        return Err(format!("{name} must be one of {allowed:?}"));
    }
    Ok(value)
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
