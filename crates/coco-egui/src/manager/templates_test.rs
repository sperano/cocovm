use std::fs;

use super::*;
use crate::machine_def::tests::TempDir;
use crate::manager::presets::Preset;
use crate::nitros9::EOU_IMAGES;

/// The date the fixture's seeded templates carry.
pub(crate) const CREATED: &str = "2026-10-23";

/// The slug the seeded NitrOS-9 EOU template gets.
pub(crate) const EOU_TEMPLATE: &str = "nitros-9-eou";

/// Temporary config and data directories holding templates.
pub(crate) struct TemplateFixture {
    config: TempDir,
    data: TempDir,
}

impl TemplateFixture {
    /// Empty template directories.
    pub(crate) fn new(tag: &str) -> Self {
        Self {
            config: TempDir::new(&format!("{tag}-config")),
            data: TempDir::new(&format!("{tag}-data")),
        }
    }

    /// Directories holding the built-in templates, without the EOU images.
    pub(crate) fn seeded(tag: &str) -> Self {
        let fixture = Self::new(tag);
        seed::seed_builtins(&fixture.dirs(), None, Some(CREATED.to_string()))
            .expect("seeding succeeds");
        fixture
    }

    pub(crate) fn dirs(&self) -> TemplateDirs {
        TemplateDirs {
            defs: Some(self.config.path().join(TEMPLATES_DIR_NAME)),
            media_root: Some(self.data.path().join(TEMPLATES_DIR_NAME)),
        }
    }

    /// The manager's template state over these directories.
    pub(crate) fn state(&self) -> TemplateState {
        TemplateState::new(self.dirs())
    }

    /// Template `slug`'s media folder.
    pub(crate) fn media_dir(&self, slug: &str) -> std::path::PathBuf {
        self.dirs().media_dir(slug).unwrap()
    }

    /// Put stand-ins for the EOU images, `len` bytes each, into the EOU
    /// template's media folder. Sparse, so a large one costs nothing to make.
    pub(crate) fn fill_eou(&self, len: u64) {
        for name in EOU_IMAGES {
            fs::File::create(self.media_dir(EOU_TEMPLATE).join(name))
                .and_then(|file| file.set_len(len))
                .unwrap();
        }
    }
}

/// A plain template named `name` in `slug`.
pub(crate) fn sample_template(slug: &str, name: &str) -> Template {
    let mut def = Preset::CoCo3DriveWire.def(None);
    def.name = name.to_string();
    Template {
        slug: slug.to_string(),
        def,
        description: String::new(),
    }
}

/// A saved template loads back with its description, which stays out of
/// the definition's unknown keys, so a machine made from it doesn't carry it.
#[test]
fn save_and_load_round_trip_the_description() {
    let fixture = TemplateFixture::new("templates-round-trip");
    let dir = fixture.dirs().defs.unwrap();
    let template = Template {
        description: "Ready for games".to_string(),
        ..sample_template("games", "Games")
    };

    save(&dir, &template).unwrap();
    let loaded = load(&dir, "games").unwrap();

    assert_eq!(loaded, template);
    assert!(loaded.def.unknown.is_empty());
    let text = fs::read_to_string(machine_def::def_path(&dir, "games")).unwrap();
    assert!(text.contains("description = \"Ready for games\""), "{text}");
}

/// An empty description writes no key, and reads back empty.
#[test]
fn empty_description_writes_no_key() {
    let fixture = TemplateFixture::new("templates-no-description");
    let dir = fixture.dirs().defs.unwrap();

    save(&dir, &sample_template("plain", "Plain")).unwrap();

    let text = fs::read_to_string(machine_def::def_path(&dir, "plain")).unwrap();
    assert!(!text.contains(DESCRIPTION_KEY), "{text}");
    assert_eq!(load(&dir, "plain").unwrap().description, "");
}

/// A template file that doesn't load is skipped, and the others still
/// list, sorted by name whatever their case.
#[test]
fn load_all_skips_a_broken_file_and_sorts_by_name() {
    let fixture = TemplateFixture::new("templates-broken");
    let dir = fixture.dirs().defs.unwrap();
    save(&dir, &sample_template("b", "beta")).unwrap();
    save(&dir, &sample_template("a", "Alpha")).unwrap();
    fs::write(machine_def::def_path(&dir, "broken"), "schema = 1\nname = ").unwrap();
    fs::write(dir.join(".hidden.toml"), "not a template").unwrap();

    let names: Vec<String> = fixture
        .dirs()
        .load_all()
        .iter()
        .map(|template| template.name().to_string())
        .collect();

    assert_eq!(names, ["Alpha", "beta"]);
}

/// A description that isn't a string fails that template's load.
#[test]
fn non_string_description_fails_the_load() {
    let fixture = TemplateFixture::new("templates-bad-description");
    let dir = fixture.dirs().defs.unwrap();
    save(&dir, &sample_template("odd", "Odd")).unwrap();
    let path = machine_def::def_path(&dir, "odd");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{DESCRIPTION_KEY} = 3\n{text}")).unwrap();

    let error = load(&dir, "odd").unwrap_err();

    assert!(error.contains(DESCRIPTION_KEY), "{error}");
}

/// A machine made from a template is dated now and starts with no stats.
#[test]
fn machine_def_resets_the_date_and_stats() {
    let mut template = sample_template("used", "Used");
    template.def.created = Some("2020-01-01".to_string());
    template.def.stats.starts = 9;

    let def = template.machine_def(Some(CREATED.to_string()));

    assert_eq!(def.created.as_deref(), Some(CREATED));
    assert_eq!(def.stats, StatsDTO::default());
    assert_eq!(def.name, "Used");
}

/// Without a description, the summary names the model and memory.
#[test]
fn summary_falls_back_to_the_hardware() {
    let template = sample_template("bare", "Bare");
    assert_eq!(template.summary(), "CoCo 3 with 512K");
    let described = Template {
        description: "Mine".to_string(),
        ..template
    };
    assert_eq!(described.summary(), "Mine");
}

/// Delete removes the file and the media folder; a template already gone
/// is fine.
#[test]
fn delete_removes_the_file_and_media() {
    let fixture = TemplateFixture::seeded("templates-delete");
    let dirs = fixture.dirs();
    let slug = "coco-3-disk-basic";
    assert!(fixture.media_dir(slug).join("disk0.dsk").is_file());

    delete(&dirs, slug).unwrap();
    delete(&dirs, slug).unwrap();

    assert!(!machine_def::def_path(dirs.defs.as_deref().unwrap(), slug).exists());
    assert!(!fixture.media_dir(slug).exists());
}

/// A slug that isn't a plain file name is refused, so a delete never
/// reaches past the templates directories.
#[test]
fn delete_refuses_a_path_slug() {
    let fixture = TemplateFixture::new("templates-delete-path");
    let error = delete(&fixture.dirs(), "../machines").unwrap_err();
    assert!(error.contains("refusing"), "{error}");
}

/// A leftover media folder keeps its slug taken.
#[test]
fn leftover_media_folder_takes_the_slug() {
    let fixture = TemplateFixture::new("templates-leftover");
    fs::create_dir_all(fixture.media_dir("old")).unwrap();
    assert!(fixture.dirs().slug_taken("old"));
    assert!(!fixture.dirs().slug_taken("new"));
}
