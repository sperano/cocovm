//! Opt-in native performance scenarios. The external runner owns XDG isolation.

use eframe::egui;
use serde_json::json;
use std::time::{Duration, Instant};

use super::ManagerApp;
mod config;
mod fixtures;
use config::Config;

const OPERATION_INTERVAL: Duration = Duration::from_secs(1);
const PERIODIC_OPERATION_SCENARIOS: &[&str] =
    &["snapshot", "lifecycle", "saved-previews", "printer"];

struct PendingScroll {
    operation: u64,
    surface: &'static str,
    position: fixtures::ScrollPosition,
    target_index: usize,
    requested: Instant,
}

pub(super) struct ScenarioRun {
    config: Config,
    started: Instant,
    measuring: Option<Instant>,
    next_operation: Instant,
    focused_updates: u64,
    unfocused_updates: u64,
    unknown_focus_updates: u64,
    operations: u64,
    operation_events: Vec<serde_json::Value>,
    scroll_phase_events: Vec<serde_json::Value>,
    pending_scroll: Option<PendingScroll>,
    scroll_error: Option<String>,
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
            operation_events: Vec::new(),
            scroll_phase_events: Vec::new(),
            pending_scroll: None,
            scroll_error: None,
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
        if let Some(error) = self.scroll_error.take() {
            return Err(error);
        }
        self.collect_printer_scroll(app)?;
        self.observe_fields(app);
        self.observe_focus(ctx);
        if now - self.measuring.expect("measurement started") >= self.config.duration {
            self.finish(app, ctx)?;
            return Ok(());
        }
        if now >= self.next_operation {
            if let Some(operation) = fixtures::operate(app, &self.config, self.operations) {
                self.record_operation(app, &operation)?;
                self.operations += 1;
                if let Err(error) = operation.outcome {
                    self.finish(app, ctx)?;
                    return Err(error);
                }
            }
            self.next_operation = now + OPERATION_INTERVAL;
        }
        let end = self.measuring.expect("measurement started") + self.config.duration;
        let next = if PERIODIC_OPERATION_SCENARIOS.contains(&self.config.name.as_str()) {
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

    fn record_operation(
        &mut self,
        app: &ManagerApp,
        attempt: &fixtures::OperationAttempt,
    ) -> Result<(), String> {
        let entry = &app.entries[0];
        let operation = &attempt.operation;
        let scroll = operation.scroll;
        self.operation_events.push(operation_event(
            attempt,
            self.operations,
            self.measuring.expect("started").elapsed(),
            entry.vm.is_some(),
            entry.suspended,
        ));
        if let Some(scroll) = scroll {
            if self.pending_scroll.is_some() {
                return Err("previous performance scroll phase did not render".into());
            }
            self.pending_scroll = Some(PendingScroll {
                operation: self.operations,
                surface: scroll.surface,
                position: scroll.position,
                target_index: scroll.target_index,
                requested: Instant::now(),
            });
        }
        Ok(())
    }

    fn collect_printer_scroll(&mut self, app: &mut ManagerApp) -> Result<(), String> {
        if self.config.name != "printer" {
            return Ok(());
        }
        let Some(result) = app.entries[0]
            .vm
            .as_mut()
            .and_then(|vm| vm.paper_window.take_perf_scroll_result())
        else {
            return Ok(());
        };
        let pending = self
            .pending_scroll
            .as_ref()
            .ok_or("printer rendered an unrequested performance scroll")?;
        if pending.operation != result.operation
            || pending.target_index != result.target_page as usize
        {
            return Err("printer rendered the wrong performance scroll request".into());
        }
        self.complete_scroll(
            result.visible_pages.start as usize..result.visible_pages.end as usize,
            result.request_to_render,
            result.draw_duration,
            result.resident_items,
            result.resident_bytes,
        )
    }

    fn complete_scroll(
        &mut self,
        actual_visible_indices: std::ops::Range<usize>,
        request_to_render: Duration,
        draw_duration: Duration,
        resident_items: usize,
        resident_bytes: usize,
    ) -> Result<(), String> {
        let Some(pending) = self.pending_scroll.take() else {
            return Ok(());
        };
        let target_visible = actual_visible_indices.contains(&pending.target_index);
        self.scroll_phase_events.push(json!({
            "event": "rendered",
            "operation": pending.operation,
            "surface": pending.surface,
            "position": pending.position.name(),
            "target_index": pending.target_index,
            "actual_visible_start_index": actual_visible_indices.start,
            "actual_visible_end_index_exclusive": actual_visible_indices.end,
            "target_visible": target_visible,
            "request_to_render_seconds": request_to_render.as_secs_f64(),
            "scroll_draw_seconds": draw_duration.as_secs_f64(),
            "resident_items": resident_items,
            "resident_bytes": resident_bytes,
            "unix_seconds": unix_seconds(),
            "measurement_elapsed_seconds": self.measuring.expect("started").elapsed().as_secs_f64(),
        }));
        if !target_visible {
            return Err(format!(
                "performance scroll target {} was outside rendered range {:?}",
                pending.target_index, actual_visible_indices
            ));
        }
        Ok(())
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
        if self.pending_scroll.is_some() {
            return Err("final performance scroll phase did not render".into());
        }
        let unix_finished = unix_seconds();
        let mut report = crate::perf::snapshot();
        report["viewport_states_at_finish"] = viewport_states(ctx);
        self.write_marker("finished", unix_finished)?;
        report["warmup_including_cold_first_update"] = self.warmup_report.take();
        report["scenario"] = json!({
            "name": self.config.name, "variant": self.config.variant,
            "display": self.config.display, "warmup_seconds": self.config.warmup.as_secs_f64(),
            "duration_seconds": self.measuring.expect("started").elapsed().as_secs_f64(),
            "saved_entries": app.entries.len(), "vm_count": app.entries.iter().filter(|e| e.vm.is_some()).count(),
            "focused_updates": self.focused_updates, "unfocused_updates": self.unfocused_updates,
            "unknown_focus_updates": self.unknown_focus_updates, "operations": self.operations,
            "operation_events": self.operation_events,
            "scroll_phase_events": self.scroll_phase_events,
            "operation_type": match self.config.name.as_str() {
                "snapshot" => "snapshot save and restore round trip",
                "lifecycle" => "named lifecycle steps covering live resume, cold resume, stop/start, window close, and restart recovery",
                "saved-previews" => "manager list scroll through beginning, middle, and end",
                "printer" => "printer paper scroll through beginning, middle, and end",
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

impl ManagerApp {
    pub(super) fn perf_manager_scroll_target(&self) -> Option<usize> {
        self.perf_scenario
            .as_ref()?
            .pending_scroll
            .as_ref()
            .filter(|scroll| scroll.surface == fixtures::MANAGER_SCROLL_SURFACE)
            .map(|scroll| scroll.target_index)
    }

    pub(super) fn complete_perf_manager_scroll(
        &mut self,
        actual_visible_indices: std::ops::Range<usize>,
        draw_duration: Duration,
    ) {
        if self.perf_manager_scroll_target().is_none() {
            return;
        }
        let resident_items = self
            .entries
            .iter()
            .filter(|entry| entry.thumbnail.is_some())
            .count();
        let resident_bytes = self
            .entries
            .iter()
            .filter_map(|entry| entry.thumbnail.as_ref())
            .map(egui::TextureHandle::byte_size)
            .sum();
        let Some(scenario) = self.perf_scenario.as_mut() else {
            return;
        };
        let Some(pending) = scenario.pending_scroll.as_ref() else {
            return;
        };
        if pending.surface != fixtures::MANAGER_SCROLL_SURFACE {
            return;
        }
        if let Err(error) = scenario.complete_scroll(
            actual_visible_indices,
            pending.requested.elapsed(),
            draw_duration,
            resident_items,
            resident_bytes,
        ) {
            scenario.scroll_error = Some(error);
        }
    }
}

fn unix_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after UNIX epoch")
        .as_secs_f64()
}

fn operation_event(
    attempt: &fixtures::OperationAttempt,
    operation_index: u64,
    measurement_elapsed: Duration,
    vm_live: bool,
    suspended: bool,
) -> serde_json::Value {
    let operation = &attempt.operation;
    let scroll = operation.scroll;
    let outcome = match &attempt.outcome {
        Ok(()) => json!("success"),
        Err(error) => json!(error),
    };
    json!({
        "name": operation.name, "operation": operation_index,
        "cycle": operation.cycle, "cycle_step": operation.cycle_step,
        "duration_seconds": attempt.duration.as_secs_f64(),
        "success": attempt.outcome.is_ok(), "outcome": outcome,
        "unix_seconds": unix_seconds(),
        "measurement_elapsed_seconds": measurement_elapsed.as_secs_f64(),
        "vm_live": vm_live, "suspended": suspended,
        "scroll_surface": scroll.map(|scroll| scroll.surface),
        "scroll_position": scroll.map(|scroll| scroll.position.name()),
        "scroll_target_index": scroll.map(|scroll| scroll.target_index),
    })
}

fn viewport_states(ctx: &egui::Context) -> serde_json::Value {
    ctx.input(|input| {
        input
            .raw
            .viewports
            .values()
            .map(|viewport| {
                json!({
                    "title": viewport.title,
                    "focused": viewport.focused,
                    "minimized": viewport.minimized,
                })
            })
            .collect::<Vec<_>>()
            .into()
    })
}

#[cfg(test)]
#[path = "perf_scenarios_test.rs"]
mod tests;
