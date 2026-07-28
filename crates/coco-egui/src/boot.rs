//! The direct-boot path: turning parsed CLI arguments into a running
//! [`CocoApp`]. The manager's equivalent, starting from a saved machine
//! definition instead, is [`crate::launch::launch_machine`].

use coco_core::{MachineConfig, MachineVariant, MonitorType};
use eframe::egui;

use crate::{
    Cli, CocoApp, DEFAULT_RTC_SLOT, DEFAULT_SSC_SLOT, MENU_BAR_H, MPI_SLOT_COUNT, RomSource, SCALE,
    STATUS_BAR_H, TARGET_ASPECT, TOOLBAR_H, default_ram, default_vdg, machine_label,
};

/// The machine the CLI flags describe. Rejected by `validate` at the call
/// site if the flags don't make sense together for the chosen variant.
pub(crate) fn config_from_cli(cli: &Cli) -> MachineConfig {
    let variant = cli.machine;
    MachineConfig {
        variant,
        video: cli.video,
        memory: cli.ram.unwrap_or_else(|| default_ram(variant)),
        // An explicit --monitor on a CoCo 1/2 flows through as Some so
        // `validate` rejects it with the real reason (no monitor port)
        // instead of silently ignoring the flag.
        monitor: match variant {
            MachineVariant::Coco3 => Some(cli.monitor.map_or(MonitorType::RGB, Into::into)),
            MachineVariant::Coco1 | MachineVariant::Coco2 => cli.monitor.map(Into::into),
        },
        // No CLI flag for this yet; same family default as the "New…"
        // dialog and the manager's detail pane (`default_vdg`).
        vdg: default_vdg(variant),
    }
}

/// Without `--mpi`, `--cart`, `--disk0`/`--disk1`/`--fd502`, `--rtc` and
/// `--ssc` all want the single cartridge port. clap's declarative
/// `conflicts_with` can't express "only when `--mpi` is absent", so the
/// combination is checked by hand.
pub(crate) fn exit_on_cartridge_port_conflict(cli: &Cli) {
    let claims = [
        cli.cart.is_some(),
        cli.disk0.is_some() || cli.disk1.is_some() || cli.fd502,
        cli.rtc,
        cli.ssc,
    ]
    .into_iter()
    .filter(|&claims| claims)
    .count();
    if !cli.mpi && claims > 1 {
        eprintln!(
            "coco: --cart, --disk0/--disk1/--fd502, --rtc, and --ssc all need the cartridge \
             port; combine them only with --mpi"
        );
        std::process::exit(1);
    }
}

/// Window geometry and chrome for the emulator window.
pub(crate) fn native_options(variant: MachineVariant) -> eframe::NativeOptions {
    // Size for the aspect-corrected (wider) image so it always fits; the
    // uncorrected image is narrower and simply leaves margin.
    let img_h = coco_core::video::FB_H as f32 * SCALE;
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/coco3-console-8bit.png"))
        .expect("embedded icon PNG is valid");
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([img_h * TARGET_ASPECT, img_h + MENU_BAR_H + TOOLBAR_H + STATUS_BAR_H])
            .with_icon(icon)
            .with_title(format!("cocovm — {}", machine_label(variant))),
        ..Default::default()
    }
}

/// The fully mounted app the CLI asked for, ready to hand to eframe.
pub(crate) fn boot_app(
    cc: &eframe::CreationContext<'_>,
    cli: Cli,
    config: MachineConfig,
    rom: Box<[u8]>,
    rom_source: RomSource,
) -> CocoApp {
    let mut app = new_app(&cli, config, rom, rom_source);
    mount_cli_hardware(&mut app, &cli);

    // --state loads before --print-capture starts (reversed from the rest of
    // this module's CLI-flag ordering): the snapshot's own config and media
    // win outright, replacing `app.machine` wholesale (see `Cli::state`'s
    // doc) — including its bit-banger, whose sink
    // `CocoApp::apply_restored_machine` resets and clears
    // `print_capture_path` unconditionally. Starting the capture AFTER that
    // means an explicit --print-capture survives the load instead of being
    // silently clobbered the instant the restored machine lands.
    if let Some(path) = cli.state {
        if let Err(e) = app.load_state_from(&path) {
            eprintln!("coco: {e}");
            std::process::exit(1);
        }
        app.refresh_window_title(&cc.egui_ctx);
    }
    if let Some(path) = cli.print_capture {
        app.start_print_capture(path);
    }
    app
}

/// With `--mpi`, `--cart`/`--disk0`/`--disk1`/`--fd502` target MPI slots
/// instead of the plain single-cartridge model, so the base constructor gets
/// none of them and [`mount_cli_hardware`] wires everything up afterward
/// through the same methods the MultiPak menu uses.
fn new_app(cli: &Cli, config: MachineConfig, rom: Box<[u8]>, rom_source: RomSource) -> CocoApp {
    let dw_paths = [cli.dw0.clone(), cli.dw1.clone(), cli.dw2.clone(), cli.dw3.clone()];
    let becker_enabled = cli.becker || dw_paths.iter().any(|p| p.is_some());
    let (cart_path, disk_paths) = if cli.mpi {
        (None, [None, None])
    } else {
        (cli.cart.clone(), [cli.disk0.clone(), cli.disk1.clone()])
    };
    CocoApp::new(
        config,
        rom,
        rom_source,
        cart_path,
        disk_paths,
        [cli.vhd0.clone(), cli.vhd1.clone()],
        dw_paths,
        becker_enabled,
        cli.hdbdos,
        cli.tape_wav,
    )
}

/// Plug in whatever the cartridge-port flags asked for — into MultiPak slots
/// when `--mpi` is set, otherwise straight into the single port.
fn mount_cli_hardware(app: &mut CocoApp, cli: &Cli) {
    let disk_paths = [cli.disk0.clone(), cli.disk1.clone()];
    if !cli.mpi {
        if cli.fd502 && let Err(e) = app.ensure_disk_controller() {
            app.cart_error = Some(e);
        } else if cli.rtc {
            app.insert_rtc();
        } else if cli.ssc {
            app.insert_ssc();
        }
        return;
    }

    app.insert_multipak();
    if let Some(path) = cli.cart.clone() {
        app.mpi_insert_rompak(0, path);
    }
    if cli.fd502 || disk_paths.iter().any(|p| p.is_some()) {
        app.mpi_insert_fd502(MPI_SLOT_COUNT - 1);
    }
    if cli.rtc {
        app.mpi_insert_rtc(DEFAULT_RTC_SLOT);
    }
    if cli.ssc {
        app.mpi_insert_ssc(DEFAULT_SSC_SLOT);
    }
    for (drive, path) in disk_paths.into_iter().enumerate() {
        if let Some(path) = path {
            app.insert_disk(drive, path);
        }
    }
}
