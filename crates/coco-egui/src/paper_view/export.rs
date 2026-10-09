//! Bounded background jobs and UI wiring for printer-paper exports.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use coco_core::dmp::DmpHandle;
use eframe::egui;

use crate::paper_export::{self, ExportError, ExportFormat, ExportRequest};

pub(super) const EXPORT_REPAINT_INTERVAL: Duration = Duration::from_millis(50);
const MAX_CONCURRENT_PRINTER_EXPORTS: usize = 2;
const EXPORT_THREAD_NAME: &str = "printer-export";
const MEBIBYTE: usize = 1024 * 1024;
/// A normal sparse multi-thousand-page roll fits while pathological output
/// cannot duplicate unbounded paper storage on the UI thread. A 15.1 MiB
/// worst-shape fixture with 110,000 distinct rows clones in 2.1 ms or less on
/// the benchmark host, leaving substantial room in a 16.7 ms frame.
const MAX_EXPORT_SNAPSHOT_BYTES: usize = 16 * MEBIBYTE;
const MAX_EXPORT_INCREMENTAL_MEMORY_BYTES: usize =
    MAX_EXPORT_SNAPSHOT_BYTES + paper_export::MAX_EXPORT_WORKING_MEMORY_BYTES;
const MAX_PROCESS_EXPORT_INCREMENTAL_MEMORY_BYTES: usize =
    MAX_CONCURRENT_PRINTER_EXPORTS * MAX_EXPORT_INCREMENTAL_MEMORY_BYTES;

static ACTIVE_EXPORTS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub(super) struct Controller {
    job: Option<Job>,
    generation: u64,
    notice: Option<String>,
}

struct Job {
    generation: u64,
    completed_pages: Arc<AtomicU32>,
    total_pages: u32,
    cancel: Arc<AtomicBool>,
    target: PathBuf,
    handle: JoinHandle<Result<(), ExportError>>,
}

impl Controller {
    pub(super) fn is_active(&self) -> bool {
        self.job.is_some()
    }

    fn start(
        &mut self,
        handle: &DmpHandle,
        target: PathBuf,
        format: ExportFormat,
        page_count: u32,
        green_bar: bool,
    ) -> Result<(), String> {
        if self.job.is_some() {
            return Err(
                "A printer export is already running. Cancel it or wait for it to finish."
                    .to_string(),
            );
        }
        let permit = ExportPermit::acquire()?;
        let paper = handle
            .paper_snapshot_with_limit(MAX_EXPORT_SNAPSHOT_BYTES)
            .map_err(|error| format!("{error}. Tear off old output or export a shorter roll."))?;
        let request = ExportRequest {
            paper,
            page_count,
            green_bar,
            target,
            format,
        };
        self.start_request(request, permit)
    }

    fn start_request(
        &mut self,
        request: ExportRequest,
        permit: ExportPermit,
    ) -> Result<(), String> {
        self.generation = self.generation.wrapping_add(1);
        self.notice = None;
        let generation = self.generation;
        let total_pages = export_page_count(&request);
        let target = request.target.clone();
        let completed_pages = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_progress = Arc::clone(&completed_pages);
        let worker_cancel = Arc::clone(&cancel);
        let handle = thread::Builder::new()
            .name(EXPORT_THREAD_NAME.to_string())
            .spawn(move || {
                let _permit = permit;
                paper_export::run(
                    request,
                    || worker_cancel.load(Ordering::Acquire),
                    |completed| worker_progress.store(completed, Ordering::Release),
                )
            })
            .map_err(|error| format!("Could not start the printer export worker: {error}"))?;
        self.job = Some(Job {
            generation,
            completed_pages,
            total_pages,
            cancel,
            target,
            handle,
        });
        Ok(())
    }

    pub(super) fn poll(&mut self) -> Option<String> {
        let finished = self
            .job
            .as_ref()
            .is_some_and(|job| job.handle.is_finished());
        if !finished {
            return None;
        }
        let job = self.job.take().expect("finished job must exist");
        if job.generation != self.generation {
            return None;
        }
        match job.handle.join() {
            Ok(Ok(())) => {
                self.notice = Some(format!("Export complete: {}", job.target.display()));
                None
            }
            Ok(Err(ExportError::Cancelled)) => {
                self.notice = Some("Printer export cancelled.".to_string());
                None
            }
            Ok(Err(error)) => Some(error.to_string()),
            Err(_) => Some("The printer export worker stopped unexpectedly.".to_string()),
        }
    }

    pub(super) fn status(&mut self, ui: &mut egui::Ui) {
        let Some(job) = &self.job else {
            if let Some(notice) = &self.notice {
                ui.separator();
                ui.label(notice);
            }
            return;
        };
        let completed = job.completed_pages.load(Ordering::Acquire);
        ui.separator();
        ui.spinner();
        ui.label(format!("Exporting {completed}/{}", job.total_pages));
        if ui.button("Cancel Export").clicked() {
            job.cancel.store(true, Ordering::Release);
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let Some(job) = self.job.take() else {
            return;
        };
        job.cancel.store(true, Ordering::Release);
        let _ = job.handle.join();
    }
}

struct ExportPermit;

impl ExportPermit {
    fn acquire() -> Result<Self, String> {
        ACTIVE_EXPORTS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_CONCURRENT_PRINTER_EXPORTS).then_some(active + 1)
            })
            .map(|_| Self)
            .map_err(|_| {
                format!(
                    "{MAX_CONCURRENT_PRINTER_EXPORTS} printer exports are already running. Wait for one to finish or cancel one. The limit caps extra export memory at {} MiB.",
                    MAX_PROCESS_EXPORT_INCREMENTAL_MEMORY_BYTES / MEBIBYTE
                )
            })
    }
}

impl Drop for ExportPermit {
    fn drop(&mut self) {
        ACTIVE_EXPORTS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn export_page_count(request: &ExportRequest) -> u32 {
    match request.format {
        ExportFormat::PagePng { .. } => 1,
        _ => request.page_count,
    }
}

pub(super) fn menu(
    ui: &mut egui::Ui,
    controller: &mut Controller,
    handle: &DmpHandle,
    current_page: u32,
    page_count: u32,
    green_bar: bool,
) -> Option<String> {
    let mut selection = None;
    ui.add_enabled_ui(!controller.is_active(), |ui| {
        ui.menu_button("Export", |ui| {
            png_menu_items(ui, current_page, &mut selection);
            ui.separator();
            pdf_menu_items(ui, &mut selection);
        });
    });
    let (target, format) = selection?;
    controller
        .start(handle, target, format, page_count, green_bar)
        .err()
}

fn png_menu_items(
    ui: &mut egui::Ui,
    current_page: u32,
    selection: &mut Option<(PathBuf, ExportFormat)>,
) {
    if ui.button("Save Page as PNG…").clicked() {
        ui.close();
        *selection = choose_png(format!("page-{}.png", current_page + 1))
            .map(|path| (path, ExportFormat::PagePng { page: current_page }));
    }
    if ui.button("Save All Pages as PNGs…").clicked() {
        ui.close();
        *selection = rfd::FileDialog::new()
            .set_file_name("printout-pages")
            .save_file()
            .map(|path| (path, ExportFormat::PagesPng));
    }
    if ui.button("Save Roll as PNG…").clicked() {
        ui.close();
        *selection = choose_png("roll.png".to_string()).map(|path| (path, ExportFormat::RollPng));
    }
}

fn pdf_menu_items(ui: &mut egui::Ui, selection: &mut Option<(PathBuf, ExportFormat)>) {
    if ui
        .button("Save as PDF (fanfold, with tractor strips)…")
        .clicked()
    {
        ui.close();
        *selection =
            choose_pdf("printout.pdf").map(|path| (path, ExportFormat::Pdf { trimmed: false }));
    }
    if ui.button("Save as PDF (trimmed, 8.5×11)…").clicked() {
        ui.close();
        *selection = choose_pdf("printout-trimmed.pdf")
            .map(|path| (path, ExportFormat::Pdf { trimmed: true }));
    }
}

fn choose_png(file_name: String) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("PNG image", &["png"])
        .set_file_name(file_name)
        .save_file()
}

fn choose_pdf(file_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("PDF document", &["pdf"])
        .set_file_name(file_name)
        .save_file()
}

#[cfg(test)]
#[path = "export_test.rs"]
mod tests;
