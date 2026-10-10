//! Native manager startup and rename recovery.

use crate::machine_def;

use super::{MachineEntry, ManagerApp, WINDOW_SIZE, assets, control, rename};

/// Open the manager as the application's main window. `config_path` is the
/// same path `main.rs` resolved `config` from (`None` when no home
/// directory exists — `paths::config_dir` docs); the Settings dialog reads
/// and writes it directly (`manager/settings.rs`), and re-levels the log
/// subscriber through `log_reload`. `machine` is the CLI's optional slug:
/// when given, that saved machine is started as the manager opens, so a
/// direct launch no longer needs the list.
pub fn run(
    config: crate::config::Config,
    config_path: Option<std::path::PathBuf>,
    log_reload: crate::startup::LogReload,
    machine: Option<String>,
) -> eframe::Result<()> {
    let crate::config::Config {
        log_level_overridden,
        control_port,
        control_port_overridden,
        assets_url,
        toolbar_icons_only,
        toolbar_icons_only_overridden,
        status_bar_icons_only,
        status_bar_icons_only_overridden,
        welcome_image_cycle,
        welcome_image_cycle_overridden,
        welcome_image_cycle_secs,
        welcome_image_cycle_secs_overridden,
        welcome_image_shuffle,
        welcome_image_shuffle_overridden,
        check_for_updates,
        hotkeys,
        manager_sort,
        ..
    } = config;
    #[cfg(target_os = "macos")]
    const ICON_BYTES: &[u8] = include_bytes!("../../assets/cocovm-icon-macos.png");
    #[cfg(not(target_os = "macos"))]
    const ICON_BYTES: &[u8] = include_bytes!("../../assets/cocovm-icon.png");
    let icon = eframe::icon_data::from_png_bytes(ICON_BYTES).expect("embedded icon PNG is valid");
    let assets_dir = crate::require_data_dir().join(crate::paths::ASSETS_DIR_NAME);
    let missing = crate::missing_assets();
    let mut viewport = crate::window_builder("CoCoVM")
        .with_inner_size(if missing.is_empty() {
            WINDOW_SIZE
        } else {
            assets::DIALOG_WINDOW_SIZE
        })
        .with_icon(icon);
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
    // Fail before any window opens when the requested slug has no definition,
    // rather than flashing the manager and exiting from inside the viewport.
    if let Some(slug) = &machine
        && !entries.iter().any(|entry| entry.slug == slug.as_str())
    {
        eprintln!("coco: no machine named '{slug}'");
        std::process::exit(1);
    }
    eframe::run_native(
        crate::APP_ID,
        options,
        Box::new(move |creation| {
            crate::banner(&crate::StartupInfo {
                roms: crate::rom_count(),
                cartridges: crate::cartridge_count(),
                machines: machine_count,
                renderer: crate::renderer_info(creation),
            });
            let control = control::bind_control(control_port, &creation.egui_ctx);
            let mut app = ManagerApp::new_with_sort(
                None,
                machines_dir,
                artifacts_root,
                entries,
                control,
                manager_sort,
            );
            app.toolbar_icons_only = toolbar_icons_only;
            app.toolbar_icons_only_overridden = toolbar_icons_only_overridden;
            app.control_port_overridden = control_port_overridden;
            app.status_bar_icons_only = status_bar_icons_only;
            app.status_bar_icons_only_overridden = status_bar_icons_only_overridden;
            app.hotkeys = hotkeys;
            let welcome = &mut app.welcome_image;
            welcome.images_dir = crate::paths::images_dir();
            welcome.cycle = welcome_image_cycle;
            welcome.cycle_overridden = welcome_image_cycle_overridden;
            welcome.cycle_secs = welcome_image_cycle_secs;
            welcome.cycle_secs_overridden = welcome_image_cycle_secs_overridden;
            welcome.shuffle = welcome_image_shuffle;
            welcome.shuffle_overridden = welcome_image_shuffle_overridden;
            welcome.load_random();
            app.config_path = config_path;
            #[cfg(target_os = "macos")]
            crate::macos_menu::install(
                &creation.egui_ctx,
                app.about_request.clone(),
                app.update_request.clone(),
            );
            if check_for_updates {
                app.update_check.start(&creation.egui_ctx, false);
            }
            app.roms_dir = crate::paths::roms_dir();
            app.cartridges_dir = crate::paths::cartridges_dir();
            app.log_reload = Some(log_reload);
            app.log_level_overridden = log_level_overridden;
            if !missing.is_empty() {
                app.asset_dialog = Some(assets::AssetDialog::new(missing, assets_url, assets_dir));
            }
            #[cfg(feature = "perf")]
            app.initialize_perf_scenario(&creation.egui_ctx)
                .map_err(std::io::Error::other)?;
            if let Some(slug) = &machine
                && let Err(error) = app.start_vm_by_slug(slug)
            {
                return Err(
                    std::io::Error::other(format!("cannot start '{slug}': {error}")).into(),
                );
            }
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

#[cfg(test)]
#[path = "run_test.rs"]
mod tests;
