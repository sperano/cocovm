//! Native manager startup and rename recovery.

use eframe::egui;

use crate::{machine_def, photo_view};

use super::{MachineEntry, ManagerApp, WINDOW_SIZE, assets, control, rename};

/// Open the manager as the application's main window. `config_path` is the
/// same path `main.rs` resolved `config` from (`None` when no home
/// directory exists — `paths::config_dir` docs); the Settings dialog reads
/// and writes it directly (`manager/settings.rs`).
pub fn run(
    config: crate::config::Config,
    config_path: Option<std::path::PathBuf>,
) -> eframe::Result<()> {
    let crate::config::Config {
        control_port,
        assets_url,
        toolbar_icons_only,
        toolbar_icons_only_overridden,
        ..
    } = config;
    const ICON_BYTE_COUNT: usize = 8_628;
    let icon_bytes: &[u8; ICON_BYTE_COUNT] = include_bytes!("../../assets/coco3-console-8bit.png");
    let icon = eframe::icon_data::from_png_bytes(icon_bytes).expect("embedded icon PNG is valid");
    let assets_dir = crate::require_data_dir().join(crate::paths::ASSETS_DIR_NAME);
    let missing = crate::missing_assets();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(if missing.is_empty() {
            WINDOW_SIZE
        } else {
            assets::DIALOG_WINDOW_SIZE
        })
        .with_icon(icon)
        .with_title("CocoVM");
    if !missing.is_empty() {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let machines_dir = machine_def::machines_dir();
    let artifacts_root = machine_def::artifacts_root();
    if let Some(dir) = machines_dir.as_deref()
        && let Err(error) = rename::recover_pending_rename(dir, artifacts_root.as_deref())
    {
        eprintln!("coco: cannot recover machine rename: {error}");
        std::process::exit(1);
    }
    let entries = load_entries(machines_dir.as_deref());
    let machine_count = entries.len();
    eframe::run_native(
        "cocovm",
        options,
        Box::new(move |creation| {
            crate::banner(&crate::StartupInfo {
                roms: crate::rom_count(),
                machines: machine_count,
                renderer: crate::renderer_info(creation),
            });
            let control = control::bind_control(control_port, &creation.egui_ctx);
            let mut app = ManagerApp::new(
                photo_view::random(),
                machines_dir,
                artifacts_root,
                entries,
                control,
            );
            app.toolbar_icons_only = toolbar_icons_only;
            app.toolbar_icons_only_overridden = toolbar_icons_only_overridden;
            app.config_path = config_path;
            if !missing.is_empty() {
                app.asset_dialog = Some(assets::AssetDialog::new(missing, assets_url, assets_dir));
            }
            #[cfg(feature = "perf")]
            app.initialize_perf_scenario(&creation.egui_ctx)
                .map_err(std::io::Error::other)?;
            Ok(Box::new(app))
        }),
    )
}

fn load_entries(dir: Option<&std::path::Path>) -> Vec<MachineEntry> {
    let definitions = match dir {
        Some(dir) => machine_def::load_all(dir),
        None => return Vec::new(),
    };
    match definitions {
        Ok(definitions) => definitions
            .into_iter()
            .map(|(slug, def)| MachineEntry::new(slug, def))
            .collect(),
        Err(error) => {
            eprintln!("coco: cannot load machine definitions: {error}");
            std::process::exit(1);
        }
    }
}
