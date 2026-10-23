use std::sync::Barrier;
use std::thread;

use super::*;
use crate::manager::templates::tests::{CREATED, EOU_TEMPLATE, TemplateFixture};

/// The slugs of the templates in `fixture`, sorted.
fn slugs(fixture: &TemplateFixture) -> Vec<String> {
    let mut slugs: Vec<String> = fixture
        .dirs()
        .load_all()
        .into_iter()
        .map(|template| template.slug)
        .collect();
    slugs.sort();
    slugs
}

/// Seeding writes every built-in template, which loads back as written,
/// with its description, its blank disk, and the seed date.
#[test]
fn seeding_writes_every_builtin_template() {
    let fixture = TemplateFixture::seeded("seed-all");

    let templates = fixture.dirs().load_all();

    assert_eq!(templates.len(), Preset::ALL.len());
    for preset in Preset::ALL {
        let template = templates
            .iter()
            .find(|template| template.name() == preset.name())
            .unwrap_or_else(|| panic!("{preset:?} is seeded"));
        assert_eq!(template.def, preset.def(Some(CREATED.to_string())));
        assert_eq!(template.description, preset.description());
        let disk = fixture
            .media_dir(&template.slug)
            .join(blank_disk_file(BLANK_DISK_DRIVE));
        assert_eq!(disk.is_file(), preset.has_blank_disk(), "{preset:?}");
    }
}

/// The New Machine dialog lists the seeded templates by name.
#[test]
fn seeded_templates_list_in_name_order() {
    let fixture = TemplateFixture::seeded("seed-order");
    let names: Vec<String> = fixture
        .dirs()
        .load_all()
        .iter()
        .map(|template| template.name().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "CoCo 2 Disk BASIC",
            "CoCo 3 Disk BASIC",
            "CoCo 3 DriveWire (HDB-DOS)",
            "NitrOS-9 EOU"
        ]
    );
}

/// A deleted built-in template stays deleted on the next launch.
#[test]
fn deleted_builtin_is_not_seeded_again() {
    let fixture = TemplateFixture::seeded("seed-deleted");
    crate::manager::templates::delete(&fixture.dirs(), "coco-3-disk-basic").unwrap();

    seed_builtins(&fixture.dirs(), None, None).unwrap();

    assert!(!slugs(&fixture).contains(&"coco-3-disk-basic".to_string()));
    assert_eq!(slugs(&fixture).len(), Preset::ALL.len() - 1);
}

/// A built-in template missing from the record, as one a later release
/// adds, is seeded on the next launch; the others are left alone, edits
/// included.
#[test]
fn builtin_missing_from_the_record_is_seeded() {
    let fixture = TemplateFixture::seeded("seed-new-release");
    let dirs = fixture.dirs();
    let defs = dirs.defs.as_deref().unwrap();
    let mut edited = crate::manager::templates::load(defs, "coco-2-disk-basic").unwrap();
    edited.def.name = "My CoCo 2".to_string();
    crate::manager::templates::save(defs, &edited).unwrap();
    crate::manager::templates::delete(&dirs, EOU_TEMPLATE).unwrap();
    let record = defs.join(SEEDED_RECORD);
    let ids = read_record(&record).unwrap();
    let without_eou: Vec<String> = ids
        .into_iter()
        .filter(|id| id != Preset::NitrOS9Eou.id())
        .collect();
    write_record(&record, &without_eou).unwrap();

    seed_builtins(&dirs, None, None).unwrap();

    assert!(slugs(&fixture).contains(&EOU_TEMPLATE.to_string()));
    let kept = crate::manager::templates::load(defs, "coco-2-disk-basic").unwrap();
    assert_eq!(kept.name(), "My CoCo 2");
    assert_eq!(read_record(&record).unwrap().len(), Preset::ALL.len());
}

/// A media folder left behind keeps its slug: the template gets the next.
#[test]
fn seeding_skips_a_leftover_media_folder() {
    let fixture = TemplateFixture::new("seed-leftover");
    std::fs::create_dir_all(fixture.media_dir("coco-3-disk-basic")).unwrap();
    std::fs::write(
        fixture.media_dir("coco-3-disk-basic").join("disk0.dsk"),
        b"old",
    )
    .unwrap();

    seed_builtins(&fixture.dirs(), None, None).unwrap();

    assert!(slugs(&fixture).contains(&"coco-3-disk-basic-2".to_string()));
    let fresh = fixture.media_dir("coco-3-disk-basic-2").join("disk0.dsk");
    assert_eq!(std::fs::metadata(fresh).unwrap().len(), 0);
}

/// The EOU images an earlier release downloaded move into the EOU
/// template's media folder, and their old folder goes.
#[test]
fn seeding_moves_legacy_eou_images() {
    let fixture = TemplateFixture::new("seed-legacy-eou");
    let legacy = crate::machine_def::tests::TempDir::new("seed-legacy-eou-cache");
    let cache = legacy.path().join("nitros9-eou");
    std::fs::create_dir_all(&cache).unwrap();
    for name in EOU_IMAGES {
        std::fs::write(cache.join(name), name).unwrap();
    }
    std::fs::write(cache.join(nitros9::LOCK_FILE), b"").unwrap();

    seed_builtins(&fixture.dirs(), Some(&cache), None).unwrap();

    for name in EOU_IMAGES {
        let moved = fixture.media_dir(EOU_TEMPLATE).join(name);
        assert_eq!(std::fs::read(moved).unwrap(), name.as_bytes());
    }
    assert!(!cache.exists());
}

/// Without a data directory there is nowhere to put media: nothing is
/// seeded, and that is not an error.
#[test]
fn seeding_needs_both_directories() {
    let fixture = TemplateFixture::new("seed-no-data");
    let dirs = TemplateDirs {
        media_root: None,
        ..fixture.dirs()
    };
    seed_builtins(&dirs, None, None).unwrap();
    assert!(dirs.load_all().is_empty());
}

/// App instances launched together seed each template once.
#[test]
fn concurrent_launches_seed_once() {
    const LAUNCHES: usize = 4;
    let fixture = TemplateFixture::new("seed-race");
    let dirs = fixture.dirs();
    let start = Barrier::new(LAUNCHES);

    let results: Vec<_> = thread::scope(|scope| {
        let launches: Vec<_> = (0..LAUNCHES)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    seed_builtins(&dirs, None, None)
                })
            })
            .collect();
        launches
            .into_iter()
            .map(|launch| launch.join().unwrap())
            .collect()
    });

    assert!(results.iter().all(Result::is_ok), "{results:?}");
    assert_eq!(slugs(&fixture).len(), Preset::ALL.len());
}
