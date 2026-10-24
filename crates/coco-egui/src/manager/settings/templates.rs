//! The Settings dialog's Templates tab: every template
//! (`manager/templates.rs`), built-in or not, with its name to edit, a
//! Delete button, and a button that shows its file in the file manager.
//! Like the rest of the dialog, nothing changes until Save: Delete marks a
//! row, its Undo clears the mark, and Cancel drops every edit, so a delete
//! needs no confirmation of its own.

use eframe::egui;

use crate::manager::reveal::FILE_MANAGER_NAME;
use crate::manager::templates::{self, Template};
use crate::manager::{ManagerApp, NO_CONFIG_DIR};

/// The tab's label.
pub(crate) const TEMPLATES_TAB_LABEL: &str = "Templates";
/// A row's buttons.
pub(crate) const DELETE_LABEL: &str = "Delete";
pub(crate) const UNDO_LABEL: &str = "Undo";
/// Above the list.
const HELP_TEXT: &str = "Rename or delete templates. Changes apply when you click Save. \
    Machines made from a template keep working when you delete it.";
/// Why Save refuses an emptied name.
const EMPTY_NAME_ERROR: &str = "Every template needs a name";

/// Width of a row's name field.
const NAME_FIELD_WIDTH: f32 = 220.0;
/// Space between two rows.
const ROW_GAP: f32 = 8.0;

/// One template's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TemplateRow {
    pub(crate) slug: String,
    /// The name in the file.
    saved_name: String,
    /// The name field's draft.
    pub(crate) name: String,
    /// The template's [`Template::summary`].
    summary: String,
    /// Marked for deletion on Save.
    pub(crate) delete: bool,
}

/// What Save applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TemplateEdits {
    /// `(slug, new name)` for each renamed template.
    pub(crate) renames: Vec<(String, String)>,
    /// The slugs of the templates to delete.
    pub(crate) deletes: Vec<String>,
}

/// The tab's draft.
#[derive(Debug, Clone, Default)]
pub(crate) struct TemplatesDraft {
    pub(crate) rows: Vec<TemplateRow>,
    /// The template whose Show in … button was clicked this frame.
    pub(crate) reveal: Option<String>,
}

impl TemplatesDraft {
    /// One unedited row per template in `templates`, in their order.
    pub(crate) fn new(templates: &[Template]) -> Self {
        Self {
            rows: templates
                .iter()
                .map(|template| TemplateRow {
                    slug: template.slug.clone(),
                    saved_name: template.name().to_string(),
                    name: template.name().to_string(),
                    summary: template.summary().into_owned(),
                    delete: false,
                })
                .collect(),
            reveal: None,
        }
    }

    /// The edits Save applies, refused when a kept template's name is empty
    /// or another kept template has it already (ignoring case), since the
    /// New Machine dialog tells templates apart by name.
    pub(crate) fn edits(&self) -> Result<TemplateEdits, String> {
        let kept: Vec<&TemplateRow> = self.rows.iter().filter(|row| !row.delete).collect();
        for (index, row) in kept.iter().enumerate() {
            let name = row.name.trim();
            if name.is_empty() {
                return Err(EMPTY_NAME_ERROR.to_string());
            }
            if kept[..index]
                .iter()
                .any(|other| templates::same_name(&other.name, name))
            {
                return Err(format!("Two templates are named “{name}”"));
            }
        }
        Ok(TemplateEdits {
            renames: kept
                .iter()
                .filter(|row| row.name.trim() != row.saved_name)
                .map(|row| (row.slug.clone(), row.name.trim().to_string()))
                .collect(),
            deletes: self
                .rows
                .iter()
                .filter(|row| row.delete)
                .map(|row| row.slug.clone())
                .collect(),
        })
    }

    /// After Save applied what it could: drop the rows of templates that
    /// are gone, and take the names now in the files. A failed edit stays
    /// in its row, for another Save.
    pub(crate) fn sync(&mut self, templates: &[Template]) {
        self.rows.retain_mut(|row| {
            let Some(template) = templates.iter().find(|t| t.slug == row.slug) else {
                return false;
            };
            template.name().clone_into(&mut row.saved_name);
            true
        });
    }

    /// The tab's contents.
    pub(crate) fn draw(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Label::new(egui::RichText::new(HELP_TEXT).weak()).wrap());
        ui.add_space(ROW_GAP);
        if self.rows.is_empty() {
            ui.weak(crate::manager::new_dialog::NO_TEMPLATES_TEXT);
            return;
        }
        for row in &mut self.rows {
            if draw_row(ui, row) {
                self.reveal = Some(row.slug.clone());
            }
            ui.add_space(ROW_GAP);
        }
    }
}

impl ManagerApp {
    /// Apply `edits` from a Settings save: the deletes, each refused while
    /// a machine uses a file in the template's folder
    /// (`ManagerApp::check_template_unused`), then the renames. Every edit
    /// is tried; the errors of those that failed come back together.
    pub(super) fn apply_template_edits(&mut self, edits: &TemplateEdits) -> Result<(), String> {
        let mut errors = Vec::new();
        for slug in &edits.deletes {
            let deleted = self
                .check_template_unused(slug)
                .and_then(|()| templates::delete(&self.templates.dirs, slug));
            errors.extend(deleted.err());
        }
        for (slug, name) in &edits.renames {
            errors.extend(self.rename_template(slug, name).err());
        }
        self.templates.reload();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("\n"))
        }
    }

    /// Rename template `slug` to `name`: its file only, the slug stays.
    fn rename_template(&self, slug: &str, name: &str) -> Result<(), String> {
        let dir = self.templates.dirs.defs.as_deref().ok_or(NO_CONFIG_DIR)?;
        let mut template = templates::load(dir, slug)?;
        template.def.name = name.to_string();
        templates::save(dir, &template)
    }

    /// Show template `slug`'s file in the file manager.
    pub(super) fn reveal_template(&self, slug: &str) -> Result<(), String> {
        let dir = self.templates.dirs.defs.as_deref().ok_or(NO_CONFIG_DIR)?;
        crate::manager::reveal::reveal(&crate::machine_def::def_path(dir, slug))
    }
}

/// One row: the name field (struck through once marked), Delete or Undo,
/// and Show in …, with the summary beneath. Returns whether Show in … was
/// clicked.
fn draw_row(ui: &mut egui::Ui, row: &mut TemplateRow) -> bool {
    let mut reveal = false;
    ui.push_id(&row.slug, |ui| {
        ui.horizontal(|ui| {
            if row.delete {
                ui.add_sized(
                    egui::vec2(NAME_FIELD_WIDTH, ui.spacing().interact_size.y),
                    egui::Label::new(egui::RichText::new(&row.name).strikethrough().weak()),
                );
            } else {
                ui.add(egui::TextEdit::singleline(&mut row.name).desired_width(NAME_FIELD_WIDTH))
                    .on_hover_text(format!("Name of the template in {}.toml", row.slug));
            }
            let toggle = if row.delete { UNDO_LABEL } else { DELETE_LABEL };
            if ui.button(toggle).clicked() {
                row.delete = !row.delete;
            }
            reveal = ui.button(format!("Show in {FILE_MANAGER_NAME}")).clicked();
        });
        ui.add(egui::Label::new(egui::RichText::new(&row.summary).weak()).wrap());
    });
    reveal
}

#[cfg(test)]
#[path = "templates_test.rs"]
mod tests;
