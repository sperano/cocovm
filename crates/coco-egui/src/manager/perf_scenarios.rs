//! Opt-in native performance scenarios. The external runner owns XDG isolation.

use eframe::egui;
use serde_json::json;
use std::time::{Duration, Instant};

use super::ManagerApp;
mod config;
mod fixtures;
use config::Config;

const OPERATION_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct ScenarioRun {
    config: Config,
    started: Instant,
    measuring: Option<Instant>,
    next_operation: Instant,
    focused_updates: u64,
    unfocused_updates: u64,
    unknown_focus_updates: u64,
    operations: u64,
    fields: Vec<(String, u64, u64)>,
    warmup_report: serde_json::Value,
    unix_started: f64,
    finished: bool,
}

impl ManagerApp {
    pub(super) fn initialize_perf_scenario(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Some(config) = Config::from_env()? else {
            return Ok(());
        };
        fixtures::prepare(self, &config)?;
        crate::perf::reset();
        if config.name != "background" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let now = Instant::now();
        self.perf_scenario = Some(ScenarioRun {
            config,
            started: now,
            measuring: None,
            next_operation: now,
            focused_updates: 0,
            unfocused_updates: 0,
            unknown_focus_updates: 0,
            operations: 0,
            fields: Vec::new(),
            warmup_report: serde_json::Value::Null,
            unix_started: 0.0,
            finished: false,
        });
        Ok(())
    }

    pub(super) fn drive_perf_scenario(&mut self, ctx: &egui::Context) {
        let Some(mut scenario) = self.perf_scenario.take() else {
            return;
        };
        if !scenario.finished
            && let Err(error) = scenario.update(self, ctx)
        {
            eprintln!("performance scenario failed: {error}");
            std::process::exit(1);
        }
        self.perf_scenario = Some(scenario);
    }
}

impl ScenarioRun {
    fn update(&mut self, app: &mut ManagerApp, ctx: &egui::Context) -> Result<(), String> {
        let now = Instant::now();
        if self.measuring.is_none() {
            let remaining = self.config.warmup.saturating_sub(now - self.started);
            if !remaining.is_zero() {
                ctx.request_repaint_after(remaining);
                return Ok(());
            }
            self.begin_measurement(app)?;
        }
        let now = Instant::now();
        self.observe_fields(app);
        self.observe_focus(ctx);
        if now - self.measuring.expect("measurement started") >= self.config.duration {
            self.finish(app, ctx)?;
            return Ok(());
        }
        if now >= self.next_operation {
            if fixtures::operate(app, &self.config, self.operations)? {
                self.operations += 1;
            }
            self.next_operation = now + OPERATION_INTERVAL;
        }
        let end = self.measuring.expect("measurement started") + self.config.duration;
        let next = if ["snapshot", "lifecycle"].contains(&self.config.name.as_str()) {
            self.next_operation.min(end)
        } else {
            end
        };
        ctx.request_repaint_after(next.saturating_duration_since(now));
        Ok(())
    }

    fn begin_measurement(&mut self, app: &ManagerApp) -> Result<(), String> {
        self.fields = app
            .entries
            .iter()
            .filter_map(|e| e.vm.as_ref().map(|v| (e.slug.clone(), v.fields_run, 0)))
            .collect();
        self.warmup_report = crate::perf::snapshot();
        self.unix_started = unix_seconds();
        self.write_marker("started", self.unix_started)?;
        crate::perf::reset();
        let now = Instant::now();
        self.measuring = Some(now);
        self.next_operation = now;
        Ok(())
    }

    fn observe_fields(&mut self, app: &ManagerApp) {
        for (slug, previous, total) in &mut self.fields {
            let current = app
                .entries
                .iter()
                .find(|e| &e.slug == slug)
                .and_then(|e| e.vm.as_ref())
                .map_or(0, |v| v.fields_run);
            *total += if current >= *previous {
                current - *previous
            } else {
                current
            };
            *previous = current;
        }
    }

    fn write_marker(&self, suffix: &str, time: f64) -> Result<(), String> {
        let mut path = self.config.output.as_os_str().to_os_string();
        path.push(format!(".{suffix}"));
        std::fs::write(path, json!({"unix_seconds": time}).to_string()).map_err(|e| e.to_string())
    }

    fn observe_focus(&mut self, ctx: &egui::Context) {
        ctx.input(|input| {
            let viewports = &input.raw.viewports;
            if viewports.values().any(|v| v.focused == Some(true)) {
                self.focused_updates += 1;
            } else if viewports.is_empty() || viewports.values().any(|v| v.focused.is_none()) {
                self.unknown_focus_updates += 1;
            } else {
                self.unfocused_updates += 1;
            }
        });
    }

    fn finish(&mut self, app: &ManagerApp, ctx: &egui::Context) -> Result<(), String> {
        let unix_finished = unix_seconds();
        let mut report = crate::perf::snapshot();
        self.write_marker("finished", unix_finished)?;
        report["warmup_including_cold_first_update"] = self.warmup_report.take();
        report["scenario"] = json!({
            "name": self.config.name, "variant": self.config.variant,
            "display": self.config.display, "warmup_seconds": self.config.warmup.as_secs_f64(),
            "duration_seconds": self.measuring.expect("started").elapsed().as_secs_f64(),
            "saved_entries": app.entries.len(), "vm_count": app.entries.iter().filter(|e| e.vm.is_some()).count(),
            "focused_updates": self.focused_updates, "unfocused_updates": self.unfocused_updates,
            "unknown_focus_updates": self.unknown_focus_updates, "operations": self.operations,
            "operation_type": match self.config.name.as_str() {
                "snapshot" => "snapshot save and restore round trip",
                "lifecycle" => "one lifecycle step: suspend, close suspended window, cold resume, stop, start, close and restart",
                _ => "none"
            },
            "measurement_unix_started": self.unix_started, "measurement_unix_finished": unix_finished,
            "fields_run": self.fields.iter().map(|(_, _, total)| total).sum::<u64>(),
            "printer_pages": if self.config.name == "printer" { config::PRINTER_PAGE_COUNT } else { 0 },
            "printer_input": "one short ink line followed by 65 blank line feeds per page",
            "vms": app.entries.iter().filter_map(|e| e.vm.as_ref().map(|v| json!({
                "slug": e.slug, "running": v.running, "suspended": e.suspended,
                "framebuffer_width": v.machine.fb_width, "audio_source_rate": v.machine.audio_sample_rate(),
                "fields_run": self.fields.iter().find(|(slug, _, _)| slug == &e.slug)
                    .map_or(0, |(_, _, total)| *total)
            }))).collect::<Vec<_>>()
        });
        let mut encoded = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
        encoded.push(b'\n');
        std::fs::write(&self.config.output, encoded).map_err(|e| e.to_string())?;
        self.finished = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }
}

fn unix_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after UNIX epoch")
        .as_secs_f64()
}
