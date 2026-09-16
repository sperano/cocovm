use std::fs;
use std::path::{Path, PathBuf};

use owo_colors::colors::xterm;
use owo_colors::{OwoColorize, Stream};
use pluralizer::pluralize;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

use crate::paths;

/// Seed the environment from a `.env` file, before anything reads it. Two
/// lookups — CWD-relative and per-user config dir — cover both a terminal
/// launch and an app-bundle launch (CWD `/`).
pub(crate) fn load_dotenv() {
    let _ = dotenvy::dotenv();
    if let Some(dir) = paths::config_dir() {
        let _ = dotenvy::from_path(dir.join(".env"));
    }
}

/// Whether stdout can carry ANSI color: the console took the VT opt-in
/// (Windows conhost; a no-op elsewhere) and stdout is a terminal.
pub(crate) fn use_color() -> bool {
    enable_ansi_support::enable_ansi_support().is_ok()
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Swaps the live log filter (`SettingsDialog`'s log level, `manager/settings.rs`).
pub(crate) type LogReload = reload::Handle<EnvFilter, Registry>;

/// Install the global log subscriber at `level`, colored only when
/// `use_color` allows it, and return the handle that re-levels it later.
pub(crate) fn setup_logging(use_color: bool, level: LevelFilter) -> LogReload {
    let (filter, handle) = reload::Layer::new(log_filter(level));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_ansi(use_color))
        .init();
    handle
}

/// The filter for `level`. `RUST_LOG` still wins when set — it filters per
/// module, which `--log-level` cannot express — so re-leveling keeps it too.
pub(crate) fn log_filter(level: LevelFilter) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(level.into())
        .from_env_lossy()
}

/// Inner width of the banner box, in columns.
const BANNER_WIDTH: usize = 74;

/// What the banner box reports below its title rule.
///
/// The graphics backend is only known once eframe has built its context, so
/// the whole box is printed from inside `run_native`'s creation closure
/// rather than at the top of `main`.
pub(crate) struct StartupInfo {
    /// ROM images installed in [`paths::roms_dir`], from [`rom_count`].
    pub roms: usize,
    /// Cartridge images installed in [`paths::cartridges_dir`], from [`cartridge_count`].
    pub cartridges: usize,
    /// Machine definitions the manager loaded.
    pub machines: usize,
    /// One-line graphics backend description, from [`renderer_info`].
    pub renderer: String,
}

impl StartupInfo {
    /// `"8 ROMs, 126 cartridges and 7 machine configurations found."`
    fn inventory(&self) -> String {
        let assets = asset_inventory(self.roms, self.cartridges);
        let machines = pluralize("machine configuration", to_isize(self.machines), true);
        format!("{assets} and {machines} found.")
    }
}

/// `"8 ROMs, 126 cartridges"` — the installed-asset half of [`StartupInfo::inventory`],
/// also printed after a bundle install.
pub(crate) fn asset_inventory(roms: usize, cartridges: usize) -> String {
    // Not `pluralize`: it upper-cases ROMS's suffix; the initialism reads as ROMs.
    let roms = format!("{roms} ROM{}", if roms == 1 { "" } else { "s" });
    let cartridges = pluralize("cartridge", to_isize(cartridges), true);
    format!("{roms}, {cartridges}")
}

/// `pluralize` takes a signed count; saturate rather than wrap.
fn to_isize(count: usize) -> isize {
    isize::try_from(count).unwrap_or(isize::MAX)
}

/// Dim `s` when stdout is a color-capable terminal, else pass it through.
fn dim(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |v| v.dimmed())
        .to_string()
}

/// Whether a directory entry names an asset with `extension`. Dotfiles are
/// rejected — macOS's AppleDouble `._name.rom` siblings would double the count.
fn is_asset_file(name: &str, extension: &str) -> bool {
    !name.starts_with('.') && name.ends_with(extension)
}

/// Whether a directory entry names a ROM image.
fn is_rom_file(name: &str) -> bool {
    is_asset_file(name, ".rom")
}

/// Whether a directory entry names a cartridge image (`.ccc`, the bundle's
/// cartridge format).
fn is_cartridge_file(name: &str) -> bool {
    is_asset_file(name, ".ccc")
}

/// How many entries of `dir` satisfy `is_asset`. A missing or unreadable
/// directory counts as zero.
fn asset_count(dir: Option<PathBuf>, is_asset: fn(&str) -> bool) -> usize {
    let Some(dir) = dir else {
        return 0;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_asset(&entry.file_name().to_string_lossy()))
        .count()
}

/// How many ROM images are installed in [`paths::roms_dir`].
pub(crate) fn rom_count() -> usize {
    asset_count(paths::roms_dir(), is_rom_file)
}

/// How many cartridge images are installed in [`paths::cartridges_dir`].
pub(crate) fn cartridge_count() -> usize {
    asset_count(paths::cartridges_dir(), is_cartridge_file)
}

/// Print one content row of the banner box, padded out to the right wall.
fn banner_row(wall: &str, text: &str) {
    // The leading space eats one of the box's inner columns.
    println!("{wall} {text:<0$}{wall}", BANNER_WIDTH - 1);
}

pub(crate) fn banner(info: &StartupInfo) {
    let fill = dim(&"═".repeat(BANNER_WIDTH));
    let wall = dim("│");
    println!("{}{fill}{}", dim("╭"), dim("╮"));
    println!(
        "{wall} CoCoVM {} {} A Tandy {}{}{} Color Computer emulator {} © 2026 Éric Spérano {wall}",
        env!("CARGO_PKG_VERSION").if_supports_color(Stream::Stdout, |v| v.cyan()),
        "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::BittersweetOrange>()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::PersianGreen>()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::ScampiIndigo>()),
        "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
    );
    // Light rule, so it reads as an inner divider rather than a box edge.
    let rule = dim(&"─".repeat(BANNER_WIDTH));
    println!("{}{rule}{}", dim("├"), dim("┤"));
    banner_row(&wall, &info.renderer);
    banner_row(&wall, &info.inventory());
    println!("{}{fill}{}", dim("╰"), dim("╯"));
}

/// Where `--assets-url` (`COCOVM_ASSETS_URL`) points unless overridden. Must
/// never contain `"` or `\` — `config::default_config_template` interpolates
/// it unescaped into a quoted TOML string.
pub(crate) const DEFAULT_ASSETS_URL: &str = "https://assets.spe.quebec/cocovm/cocovm-assets-v8.tgz";

/// ROM images the bundle at [`DEFAULT_ASSETS_URL`] carries. Any one missing from
/// the installed ROM directory triggers a (re)download, so an install that
/// predates a bundle addition catches up instead of staying at whatever it
/// first unpacked.
pub(crate) const BUNDLED_ROMS: [&str; 13] = [
    "bas10.rom",
    "bas11.rom",
    "bas12.rom",
    "bas13.rom",
    "extbas10.rom",
    "extbas11.rom",
    "coco3.rom",
    "disk11.rom",
    "sp0256-al2.rom",
    "ssc-tms7040.rom",
    "hdbdw3bc3.rom",
    "rs232.rom",
    "orch90.rom",
];

/// Whether `dir` exists and contains at least one entry.
pub(crate) fn dir_has_files(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// The [`BUNDLED_ROMS`] not present as files under `roms_dir`.
pub(crate) fn missing_bundled_roms(roms_dir: &Path) -> Vec<&'static str> {
    BUNDLED_ROMS
        .into_iter()
        .filter(|name| !roms_dir.join(name).is_file())
        .collect()
}

/// Unpack a gzipped tar stream into `dest`. Split from the download so the
/// extraction can be unit-tested without a network.
pub(crate) fn unpack_assets(reader: impl std::io::Read, dest: &Path) -> std::io::Result<()> {
    let gz = flate2::read::GzDecoder::new(reader);
    tar::Archive::new(gz).unpack(dest)
}

/// Download the bundle at `url` and unpack it into `dest`, streaming — the
/// tarball is never held in memory or written to disk whole.
pub(crate) fn download_and_unpack_assets(
    url: &str,
    dest: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dest)?;
    let response = ureq::get(url).call()?;
    unpack_assets(response.into_body().into_reader(), dest)?;
    Ok(())
}

/// The per-user data directory, or a fatal exit when no home directory
/// exists — everything downstream (`rom_load::installed_roms_dir`) relies
/// on this check having passed at startup.
pub(crate) fn require_data_dir() -> std::path::PathBuf {
    let Some(data_dir) = paths::data_dir() else {
        eprintln!("no home directory found; cannot locate the asset directories");
        std::process::exit(1);
    };
    data_dir
}

/// The asset files the bundle at [`DEFAULT_ASSETS_URL`] should provide but which
/// are absent on disk, as display paths: an empty images directory counts
/// as one entry, plus each missing [`BUNDLED_ROMS`] image. Empty means no
/// download is needed. The integration tests' disk images are not the
/// app's business: `crates/test-assets` fetches its own bundle.
pub(crate) fn missing_assets() -> Vec<String> {
    let mut missing: Vec<String> = paths::images_dir()
        .filter(|dir| !dir_has_files(dir))
        .map(|dir| dir.display().to_string())
        .into_iter()
        .collect();
    if let Some(roms_dir) = paths::roms_dir() {
        missing.extend(
            missing_bundled_roms(&roms_dir)
                .into_iter()
                .map(|name| roms_dir.join(name).display().to_string()),
        );
    }
    missing
}

/// Describe which graphics backend eframe actually created, and on what
/// GPU, as one banner-sized line. eframe has no backend-name API, so this
/// matches on which `CreationContext` handle is present and uses that
/// backend's own introspection.
pub(crate) fn renderer_info(cc: &eframe::CreationContext<'_>) -> String {
    #[cfg(feature = "wgpu")]
    if let Some(render_state) = cc.wgpu_render_state.as_ref() {
        let info = render_state.adapter.get_info();
        return format!(
            "Renderer: {:?} on {} ({:?}).",
            info.backend, info.name, info.device_type
        );
    }
    if let Some(gl) = cc.gl.as_ref() {
        use eframe::glow::HasContext as _;
        let api = if gl.version().is_embedded {
            "OpenGL ES"
        } else {
            "OpenGL"
        };
        // Safety: eframe made this context current for the creation closure; VERSION/RENDERER
        // are valid glGetString enums.
        let (version, renderer) = unsafe {
            (
                gl.get_parameter_string(eframe::glow::VERSION),
                gl.get_parameter_string(eframe::glow::RENDERER),
            )
        };
        return format!("{api} version: {version} ({renderer}).");
    }
    "Renderer: unknown backend.".to_string()
}

#[cfg(test)]
#[path = "startup_test.rs"]
mod tests;
