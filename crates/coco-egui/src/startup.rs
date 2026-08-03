use std::fs;
use std::path::Path;

use owo_colors::colors::xterm;
use owo_colors::{OwoColorize, Stream};
use pluralizer::pluralize;
use tracing_subscriber::filter::LevelFilter;

use crate::paths;

/// Seed the environment from a `.env` file, before anything reads it.
///
/// Two lookups, because the two ways the app starts have different working
/// directories: from a terminal, `dotenv` walks up from the CWD, which finds
/// a checkout's own `.env`; from Finder or an `.app` bundle the CWD is `/`,
/// so that walk finds nothing and the per-user config directory answers
/// instead. Real environment variables always win — neither call overwrites
/// a key that is already set — and a missing file is not an error.
pub(crate) fn load_dotenv() {
    let _ = dotenvy::dotenv();
    if let Some(dir) = paths::config_dir() {
        let _ = dotenvy::from_path(dir.join(".env"));
    }
}

/// Whether stdout can carry ANSI color: the console took the VT opt-in and
/// stdout is a terminal.
///
/// Legacy Windows conhost only interprets escape codes after the app opts in
/// — a no-op everywhere else — so this has to run before anything colors
/// stdout. [`banner`] depends on the opt-in too, but not on this return
/// value: `owo_colors`' `if_supports_color` detects the terminal itself.
pub(crate) fn use_color() -> bool {
    enable_ansi_support::enable_ansi_support().is_ok()
        && std::io::IsTerminal::is_terminal(&std::io::stdout())
}

/// Install the global log subscriber: leveled stdout logging at `level`,
/// colored only when `use_color` says stdout can take it.
///
/// `level` is only the *default* directive — `RUST_LOG` still wins when set,
/// because it can filter per module (`RUST_LOG=info,eframe=warn`,
/// `RUST_LOG=coco_egui::audio=debug`), which `--log-level` cannot express.
pub(crate) fn setup_logging(use_color: bool, level: LevelFilter) {
    tracing_subscriber::fmt()
        .with_ansi(use_color)
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(level.into())
                .from_env_lossy(),
        )
        .init();
}

/// Inner width of the banner box, in columns.
const BANNER_WIDTH: usize = 76;

/// What the banner box reports below its title rule.
///
/// The graphics backend is only known once eframe has built its context, so
/// the whole box is printed from inside `run_native`'s creation closure
/// rather than at the top of `main`.
pub(crate) struct StartupInfo {
    /// ROM images installed in [`paths::roms_dir`], from [`rom_count`].
    pub roms: usize,
    /// Machine definitions the manager loaded. `None` on the direct-boot
    /// path, which never reads the machine list.
    pub machines: Option<usize>,
    /// One-line graphics backend description, from [`renderer_info`].
    pub renderer: String,
}

impl StartupInfo {
    /// `"8 ROMs and 7 machine configurations found."` — the machine half is
    /// dropped when there is no machine list to speak of.
    fn inventory(&self) -> String {
        // Not `pluralize`: it upper-cases the suffix of an all-caps acronym
        // ("ROMS"), and the initialism reads as "ROMs".
        let roms = format!("{} ROM{}", self.roms, if self.roms == 1 { "" } else { "s" });
        match self.machines {
            Some(n) => {
                let machines = pluralize("machine configuration", n as isize, true);
                format!("{roms} and {machines} found.")
            }
            None => format!("{roms} found."),
        }
    }
}

/// Dim `s` when stdout is a color-capable terminal, else pass it through.
fn dim(s: &str) -> String {
    s.if_supports_color(Stream::Stdout, |v| v.dimmed())
        .to_string()
}

/// Whether a directory entry names a ROM image.
///
/// Dotfiles are rejected: unpacking the asset tarball on macOS leaves an
/// AppleDouble `._name.rom` beside every real ROM, which would double the
/// count.
fn is_rom_file(name: &str) -> bool {
    !name.starts_with('.') && name.ends_with(".rom")
}

/// How many ROM images are installed in [`paths::roms_dir`].
///
/// A missing or unreadable directory simply counts as zero.
pub(crate) fn rom_count() -> usize {
    let Some(dir) = paths::roms_dir() else {
        return 0;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_rom_file(&entry.file_name().to_string_lossy()))
        .count()
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
        "{wall} CoCoVM v{} {} A Tandy {}{}{} Color Computers emulator {} © 2026 Éric Spérano {wall}",
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

pub(crate) const ASSETS_URL: &str = "https://assets.spe.quebec/cocovm-assets-v2.tgz";

/// Whether `dir` exists and contains at least one entry.
pub(crate) fn dir_has_files(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

/// Unpack a gzipped tar stream into `dest`. Split from the download so the
/// extraction can be unit-tested without a network.
pub(crate) fn unpack_assets(reader: impl std::io::Read, dest: &Path) -> std::io::Result<()> {
    let gz = flate2::read::GzDecoder::new(reader);
    tar::Archive::new(gz).unpack(dest)
}

/// Download [`ASSETS_URL`] and unpack it into `dest`, streaming — the
/// tarball is never held in memory or written to disk whole.
pub(crate) fn download_and_unpack_assets(dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dest)?;
    let response = ureq::get(ASSETS_URL).call()?;
    unpack_assets(response.into_body().into_reader(), dest)?;
    Ok(())
}

pub(crate) fn ensure_assets() {
    let Some(data_dir) = paths::data_dir() else {
        eprintln!("no home directory found; cannot locate the asset directories");
        std::process::exit(1);
    };
    let missing: Vec<String> = [paths::roms_dir(), paths::images_dir()]
        .into_iter()
        .flatten()
        .filter(|dir| !dir_has_files(dir))
        .map(|dir| dir.display().to_string())
        .collect();
    if missing.is_empty() {
        return;
    }
    println!("Downloading {ASSETS_URL}…");
    match download_and_unpack_assets(&data_dir) {
        Ok(()) => println!("assets installed in {}", data_dir.display()),
        Err(e) => eprintln!("asset download failed: {e}"),
    }
}

/// Describe which graphics backend eframe actually created, and on what GPU,
/// as one banner-sized line.
///
/// eframe has no backend-name API: `CreationContext` carries one handle per
/// compiled backend (`gl` for glow, `wgpu_render_state` behind the `wgpu`
/// feature) and the *presence* of a handle is the portable signal — so this
/// matches on the handles rather than assuming a backend. Each arm then
/// uses that backend's own introspection: wgpu's `AdapterInfo` names the
/// API and GPU directly; glow's cached [`eframe::glow::Version`] (a safe
/// call) distinguishes OpenGL from OpenGL ES, with only the GPU-name
/// string needing a raw `glGetString`.
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
        // Safety: eframe made this context current on this thread for the
        // duration of the creation closure, and VERSION/RENDERER are valid
        // `glGetString` enums.
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
