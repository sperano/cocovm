//! Machine templates: the definitions the New Machine dialog creates
//! machines from. A template is a file, `config_dir()/templates/<slug>.toml`,
//! in the machine definition schema plus a top-level [`DESCRIPTION_KEY`],
//! and a media folder, `data_dir()/templates/<slug>/`, where its relative
//! paths resolve.
//!
//! The built-in templates are written there once (`seed.rs`) and are
//! ordinary files from then on: the Settings dialog's Templates tab renames
//! and deletes any of them (`manager/settings/templates.rs`), and a powered
//! off machine can be saved as a new one (`manager/save_template.rs`).
//!
//! A machine made from a template is independent of it: it gets its own
//! copy of the template's relative media (`media.rs`), and its absolute
//! paths stay references to the same host files. A template file that fails
//! to load is skipped with a warning, never fatal like a broken machine
//! definition: a template is only a starting point.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::machine_def::{self, MachineDef, StatsDTO};
use crate::paths;

pub(crate) mod media;
pub(crate) mod seed;

/// The top-level key a template file adds to the machine definition schema:
/// the text the New Machine dialog shows under the template's name.
const DESCRIPTION_KEY: &str = "description";

/// Name of the templates directory under the config dir, and of the media
/// root under the data dir.
const TEMPLATES_DIR_NAME: &str = "templates";

/// One template file, loaded.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Template {
    /// The file stem. A rename changes only the name, never the slug.
    pub(crate) slug: String,
    /// The definition machines made from the template start as. Its `name`
    /// is the template's name, which each machine takes too.
    pub(crate) def: MachineDef,
    /// [`DESCRIPTION_KEY`]'s value; empty when the file has none.
    pub(crate) description: String,
}

impl Template {
    /// The template's name.
    pub(crate) fn name(&self) -> &str {
        &self.def.name
    }

    /// The definition of a machine made from this template now: the
    /// template's, dated `created`, with zeroed `[stats]`.
    pub(crate) fn machine_def(&self, created: Option<String>) -> MachineDef {
        MachineDef {
            created,
            stats: StatsDTO::default(),
            ..self.def.clone()
        }
    }

    /// The text under the template's name: its description, or, without
    /// one, its model and memory.
    pub(crate) fn summary(&self) -> Cow<'_, str> {
        if !self.description.trim().is_empty() {
            return Cow::Borrowed(&self.description);
        }
        let hardware = &self.def.hardware;
        Cow::Owned(format!(
            "{} with {}",
            crate::machine_label(hardware.variant.into()),
            crate::new_vm::ram_label(hardware.ram.into())
        ))
    }
}

/// Where the templates live. Injected like `ManagerApp::machines_dir`, so
/// tests use temporary directories; `None` when no home directory exists
/// (`paths::config_dir` docs).
#[derive(Debug, Clone, Default)]
pub(crate) struct TemplateDirs {
    /// `config_dir()/templates`, holding the template files.
    pub(crate) defs: Option<PathBuf>,
    /// `data_dir()/templates`, holding each template's media folder.
    pub(crate) media_root: Option<PathBuf>,
}

impl TemplateDirs {
    /// The user's real template directories.
    pub(crate) fn user() -> Self {
        Self {
            defs: paths::config_dir().map(|dir| dir.join(TEMPLATES_DIR_NAME)),
            media_root: paths::data_dir().map(|dir| dir.join(TEMPLATES_DIR_NAME)),
        }
    }

    /// Template `slug`'s media folder.
    pub(crate) fn media_dir(&self, slug: &str) -> Option<PathBuf> {
        self.media_root.as_ref().map(|root| root.join(slug))
    }

    /// Whether `candidate` already names a template file or a media folder.
    /// A media folder can outlive its file (a hand-removed `.toml`), and a
    /// new template must never start with its files.
    pub(crate) fn slug_taken(&self, candidate: &str) -> bool {
        machine_def::slug_on_disk(self.defs.as_deref(), self.media_root.as_deref(), candidate)
    }

    /// Every template file in [`Self::defs`], sorted by name. A file that
    /// fails to load is skipped with a warning.
    pub(crate) fn load_all(&self) -> Vec<Template> {
        let Some(dir) = &self.defs else {
            return Vec::new();
        };
        let mut templates: Vec<Template> = template_slugs(dir)
            .into_iter()
            .filter_map(|slug| match load(dir, &slug) {
                Ok(template) => Some(template),
                Err(error) => {
                    tracing::warn!("skipping template: {error}");
                    None
                }
            })
            .collect();
        sort_by_name(&mut templates);
        templates
    }
}

/// The manager's templates: where they live, the list loaded from there,
/// and the work on them in flight.
#[derive(Default)]
pub(crate) struct TemplateState {
    pub(crate) dirs: TemplateDirs,
    /// Every loaded template, sorted by name. Reloaded when the New Machine
    /// dialog or the Settings dialog opens, so hand edits show up.
    pub(crate) list: Vec<Template>,
    /// Machines being written from templates on their own threads
    /// (`manager/template_install.rs`).
    pub(crate) installs: Vec<super::template_install::PendingInstall>,
    /// The Save as Template dialog (`manager/save_template.rs`), while open.
    pub(crate) save_dialog: Option<super::save_template::SaveTemplateDialog>,
    /// The machine whose Save as Template was clicked, opened before the
    /// next frame draws (`ManagerApp::open_requested_save_template`).
    pub(crate) save_request: Option<String>,
}

impl TemplateState {
    /// The templates in `dirs`, loaded.
    pub(crate) fn new(dirs: TemplateDirs) -> Self {
        Self {
            list: dirs.load_all(),
            dirs,
            ..Self::default()
        }
    }

    /// Load the list from disk again.
    pub(crate) fn reload(&mut self) {
        self.list = self.dirs.load_all();
    }

    /// The listed template `slug`.
    pub(crate) fn find(&self, slug: &str) -> Option<&Template> {
        self.list.iter().find(|template| template.slug == slug)
    }
}

/// Sort `templates` by name, ignoring case, then by slug.
fn sort_by_name(templates: &mut [Template]) {
    templates.sort_by_cached_key(|template| (name_key(template.name()), template.slug.clone()));
}

/// `name` as templates are told apart: trimmed, ignoring case.
fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Whether two template names are the same one, by [`name_key`]: Save as
/// Template replaces such a template, and the Settings dialog refuses two.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    name_key(a) == name_key(b)
}

/// The slugs of the template files directly in `dir` (dotfiles skipped).
/// A missing or unreadable `dir` has none.
fn template_slugs(dir: &Path) -> Vec<String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            tracing::warn!("{}: {e}", dir.display());
            return Vec::new();
        }
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) == Some(machine_def::DEF_EXTENSION)
        })
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_string))
        .filter(|stem| !stem.starts_with('.'))
        .collect()
}

/// Load template `slug` from `dir`.
pub(crate) fn load(dir: &Path, slug: &str) -> Result<Template, String> {
    let path = machine_def::def_path(dir, slug);
    let (def, extras) = machine_def::load_with_extras(&path, &[DESCRIPTION_KEY])?;
    let description = match extras.get(DESCRIPTION_KEY) {
        None => String::new(),
        Some(toml::Value::String(text)) => text.clone(),
        Some(_) => {
            return Err(format!(
                "{}: {DESCRIPTION_KEY} must be a string",
                path.display()
            ));
        }
    };
    Ok(Template {
        slug: slug.to_string(),
        def,
        description,
    })
}

/// Write `template` into `dir` as `<slug>.toml`, atomically
/// (`machine_def::save`). An empty description writes no key.
pub(crate) fn save(dir: &Path, template: &Template) -> Result<(), String> {
    let mut extras = toml::Table::new();
    let description = template.description.trim();
    if !description.is_empty() {
        extras.insert(
            DESCRIPTION_KEY.to_string(),
            toml::Value::String(description.to_string()),
        );
    }
    machine_def::save_with_extras(dir, &template.slug, &template.def, &extras)
}

/// Delete template `slug`: its file first, so a failure never leaves a
/// listed template without its media, then its media folder. Machines made
/// from it keep working: they have their own copies.
pub(crate) fn delete(dirs: &TemplateDirs, slug: &str) -> Result<(), String> {
    let dir = dirs.defs.as_deref().ok_or(super::NO_CONFIG_DIR)?;
    if Path::new(slug).file_name() != Some(OsStr::new(slug)) {
        return Err(format!("refusing to delete the template {slug:?}"));
    }
    let path = machine_def::def_path(dir, slug);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let Some(media_dir) = dirs.media_dir(slug) else {
        return Ok(());
    };
    match fs::remove_dir_all(&media_dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", media_dir.display())),
    }
}

#[cfg(test)]
#[path = "templates_test.rs"]
pub(crate) mod tests;
