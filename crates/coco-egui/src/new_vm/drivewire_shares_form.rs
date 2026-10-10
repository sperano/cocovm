//! The DriveWire tab's "Host shares" rows: named host folders, their
//! access, problems with either, and a running VM's share session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coco_core::drivewire::share::{
    MAX_SHARES, ShareConfigError, ShareError, ShareStatus, validate_share_name,
};
use eframe::egui;

use super::{FORM_GRID_SPACING, MachineForm};
use crate::machine_def::{DriveWireShareDTO, ShareAccessDTO, resolve_media_path};

const NAME_FIELD_WIDTH: f32 = 96.0;
const ACCESS_COMBO_WIDTH: f32 = 96.0;
const BROWSE_BUTTON_WIDTH: f32 = 76.0;
const REMOVE_BUTTON_WIDTH: f32 = 22.0;
const MIN_FOLDER_FIELD_WIDTH: f32 = 80.0;
/// Gaps between the five widgets of one share row.
const ROW_GAPS: f32 = 4.0;
/// How long a folder check stays fresh while the tab is open.
const ROOT_RECHECK_INTERVAL: Duration = Duration::from_secs(2);
const NEW_SHARE_PREFIX: &str = "share";
const EMPTY_FOLDER_HINT: &str = "No folder";
const NO_FOLDER_WARNING: &str = "No folder selected; this share is inactive.";
const SHARES_HINT: &str = "Shares name host folders that DriveWire guest services can reach. \
    Guests cannot leave a share's folder, and only read/write shares accept writes. \
    No guest service uses shares yet.";
const READ_ONLY_LABEL: &str = "Read-only";
const READ_WRITE_LABEL: &str = "Read/write";

/// Cached folder checks, so drawing the tab does not touch the filesystem
/// every frame.
#[derive(Default)]
pub(crate) struct ShareRootChecks {
    checked: HashMap<PathBuf, (Instant, Option<String>)>,
}

impl ShareRootChecks {
    /// The problem with `root`, if any, rechecked after [`ROOT_RECHECK_INTERVAL`].
    fn problem(&mut self, root: &Path) -> Option<String> {
        let now = Instant::now();
        let fresh = self
            .checked
            .get(root)
            .is_some_and(|(at, _)| now.duration_since(*at) < ROOT_RECHECK_INTERVAL);
        if !fresh {
            self.checked
                .insert(root.to_path_buf(), (now, root_problem(root)));
        }
        self.checked[root].1.clone()
    }

    fn retain(&mut self, roots: &[PathBuf]) {
        self.checked.retain(|root, _| roots.contains(root));
    }
}

fn root_problem(root: &Path) -> Option<String> {
    match std::fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => None,
        Ok(_) => Some(format!("{} is not a folder.", root.display())),
        Err(error) => Some(format!("{}: {}", root.display(), ShareError::from(error))),
    }
}

impl MachineForm {
    /// One row per share, then Add share, the running session, and the hint.
    pub(crate) fn drivewire_share_rows(
        &mut self,
        ui: &mut egui::Ui,
        slug: &str,
        session: Option<&ShareStatus>,
    ) {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = FORM_GRID_SPACING[1];
            ui.add_enabled_ui(self.drivewire.enabled, |ui| {
                ui.push_id((self.salt, "drivewire_shares"), |ui| {
                    self.share_list(ui, slug)
                });
            });
            if let Some(session) = session.filter(|_| self.drivewire.enabled) {
                session_rows(ui, session);
            }
            ui.small(SHARES_HINT);
        });
    }

    fn share_list(&mut self, ui: &mut egui::Ui, slug: &str) {
        let mut remove = None;
        for index in 0..self.drivewire.shares.len() {
            ui.push_id(index, |ui| {
                if share_row(ui, index, &mut self.drivewire.shares[index]) {
                    remove = Some(index);
                }
            });
            let shares = &self.drivewire.shares;
            let problem = share_problem(&shares[index], shares, slug, &mut self.share_roots);
            if let Some(problem) = problem {
                ui.colored_label(ui.visuals().warn_fg_color, problem);
            }
        }
        if let Some(index) = remove {
            self.drivewire.shares.remove(index);
        }
        let roots: Vec<PathBuf> = self
            .drivewire
            .shares
            .iter()
            .map(|share| resolve_media_path(&share.path, slug))
            .collect();
        self.share_roots.retain(&roots);
        let can_add = self.drivewire.shares.len() < MAX_SHARES;
        if ui
            .add_enabled(can_add, egui::Button::new("Add share"))
            .clicked()
        {
            let name = unused_share_name(&self.drivewire.shares);
            self.drivewire.shares.push(DriveWireShareDTO {
                name,
                ..DriveWireShareDTO::default()
            });
        }
    }
}

/// Name, folder, Browse, access, and remove. Returns whether remove was clicked.
fn share_row(ui: &mut egui::Ui, index: usize, share: &mut DriveWireShareDTO) -> bool {
    let number = index + 1;
    let height = ui.spacing().interact_size.y;
    let folder_width = (ui.available_width()
        - NAME_FIELD_WIDTH
        - BROWSE_BUTTON_WIDTH
        - ACCESS_COMBO_WIDTH
        - REMOVE_BUTTON_WIDTH
        - ROW_GAPS * ui.spacing().item_spacing.x)
        .max(MIN_FOLDER_FIELD_WIDTH);
    ui.horizontal(|ui| {
        let name = ui.add_sized(
            [NAME_FIELD_WIDTH, height],
            egui::TextEdit::singleline(&mut share.name).id_salt("name"),
        );
        label_text_edit(ui, &name, &share.name, format!("Share {number} name"));
        let folder = ui.add_sized(
            [folder_width, height],
            egui::TextEdit::singleline(&mut share.path)
                .id_salt("path")
                .hint_text(EMPTY_FOLDER_HINT),
        );
        label_text_edit(ui, &folder, &share.path, format!("Share {number} folder"));
        let browse = ui.add_sized([BROWSE_BUTTON_WIDTH, height], egui::Button::new("Browse…"));
        label_button(ui, &browse, format!("Browse share {number} folder…"));
        if browse.clicked()
            && let Some(selected) = folder_dialog(&share.path).pick_folder()
        {
            share.path = selected.to_string_lossy().into_owned();
        }
        access_combo(ui, number, &mut share.access);
        let remove = ui
            .add_sized([REMOVE_BUTTON_WIDTH, height], egui::Button::new("×"))
            .on_hover_text(format!("Remove share {number}"));
        label_button(ui, &remove, format!("Remove share {number}"));
        remove.clicked()
    })
    .inner
}

fn access_combo(ui: &mut egui::Ui, number: usize, access: &mut ShareAccessDTO) {
    let label = |access: ShareAccessDTO| match access {
        ShareAccessDTO::ReadOnly => READ_ONLY_LABEL,
        ShareAccessDTO::ReadWrite => READ_WRITE_LABEL,
    };
    let combo = egui::ComboBox::from_id_salt("access")
        .width(ACCESS_COMBO_WIDTH)
        .selected_text(label(*access))
        .show_ui(ui, |ui| {
            for choice in [ShareAccessDTO::ReadOnly, ShareAccessDTO::ReadWrite] {
                ui.selectable_value(access, choice, label(choice));
            }
        });
    combo.response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ComboBox,
            ui.is_enabled(),
            format!("Share {number} access"),
        )
    });
}

fn label_text_edit(ui: &egui::Ui, response: &egui::Response, text: &str, label: String) {
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::text_edit(ui.is_enabled(), text, text, "");
        info.label = Some(label.clone());
        info
    });
}

fn label_button(ui: &egui::Ui, response: &egui::Response, label: String) {
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label.clone())
    });
}

/// The first problem worth showing under a share row.
fn share_problem(
    share: &DriveWireShareDTO,
    all: &[DriveWireShareDTO],
    slug: &str,
    roots: &mut ShareRootChecks,
) -> Option<String> {
    if let Err(error) = validate_share_name(&share.name) {
        return Some(error.to_string());
    }
    let same_name = all
        .iter()
        .filter(|other| other.name.eq_ignore_ascii_case(&share.name))
        .count();
    if same_name > 1 {
        return Some(ShareConfigError::DuplicateName(share.name.clone()).to_string());
    }
    if share.path.trim().is_empty() {
        return Some(NO_FOLDER_WARNING.to_string());
    }
    roots.problem(&resolve_media_path(&share.path, slug))
}

/// `share1`, `share2`, … — the first name no share uses, ignoring case.
fn unused_share_name(shares: &[DriveWireShareDTO]) -> String {
    (1..)
        .map(|n| format!("{NEW_SHARE_PREFIX}{n}"))
        .find(|name| {
            !shares
                .iter()
                .any(|share| share.name.eq_ignore_ascii_case(name))
        })
        .expect("an unbounded sequence has an unused name")
}

fn session_rows(ui: &mut egui::Ui, session: &ShareStatus) {
    ui.label(format!(
        "Running session: current directory {}, {} open file{}.",
        session.cwd,
        session.open_handles,
        if session.open_handles == 1 { "" } else { "s" },
    ));
    if let Some(error) = session.last_error {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "Last share error: {error} (dw code {}).",
                error.command_code()
            ),
        );
    }
}

fn folder_dialog(path: &str) -> rfd::FileDialog {
    let dialog = rfd::FileDialog::new();
    let path = Path::new(path);
    if path.is_dir() {
        dialog.set_directory(path)
    } else {
        dialog
    }
}

#[cfg(test)]
#[path = "drivewire_shares_form_test.rs"]
mod tests;
