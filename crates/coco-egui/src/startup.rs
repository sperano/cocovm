use std::fs;
use std::path::Path;

use owo_colors::colors::xterm;
use owo_colors::{OwoColorize, Stream};

use crate::paths;

pub(crate) fn setup_logging() {
    // Legacy Windows conhost only interprets VT escape codes after the app
    // opts in; a no-op everywhere else. On failure, fall back to plain text.
    let vt_ok = enable_ansi_support::enable_ansi_support().is_ok();
    let use_color = vt_ok && std::io::IsTerminal::is_terminal(&std::io::stdout());
    // Leveled stdout logging, colored only when stdout is a terminal.
    // `RUST_LOG` filters per module (e.g. `RUST_LOG=info,eframe=warn` or
    // `RUST_LOG=coco_egui::audio=debug`); without it, only `warn` and above
    // is shown.
    tracing_subscriber::fmt()
        .with_ansi(use_color)
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::WARN.into())
                .from_env_lossy(),
        )
        .init();
}

pub(crate) fn banner() {
    let sep = "─".repeat(76);
    println!(
        "{}{}{}\n{} CoCoVM v{} {} A Tandy {}{}{} Color Computers emulator {} © 2026 Éric Spérano {}\n{}{}{}",
        "╭".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        sep.if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "╮".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "│".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        env!("CARGO_PKG_VERSION").if_supports_color(Stream::Stdout, |v| v.cyan()),
        "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::BittersweetOrange>()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::PersianGreen>()),
        "/".if_supports_color(Stream::Stdout, |v| v.fg::<xterm::ScampiIndigo>()),
        "-".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "│".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "╰".if_supports_color(Stream::Stdout, |v| v.dimmed()),
        sep.if_supports_color(Stream::Stdout, |v| v.dimmed()),
        "╯".if_supports_color(Stream::Stdout, |v| v.dimmed()),
    );
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

/// Print which graphics backend eframe actually created, and on what GPU.
///
/// eframe has no backend-name API: `CreationContext` carries one handle per
/// compiled backend (`gl` for glow, `wgpu_render_state` behind the `wgpu`
/// feature) and the *presence* of a handle is the portable signal — so this
/// matches on the handles rather than assuming a backend. Each arm then
/// uses that backend's own introspection: wgpu's `AdapterInfo` names the
/// API and GPU directly; glow's cached [`eframe::glow::Version`] (a safe
/// call) distinguishes OpenGL from OpenGL ES, with only the GPU-name
/// string needing a raw `glGetString`.
pub(crate) fn log_renderer_info(cc: &eframe::CreationContext<'_>) {
    #[cfg(feature = "wgpu")]
    if let Some(render_state) = cc.wgpu_render_state.as_ref() {
        let info = render_state.adapter.get_info();
        println!(
            "Renderer: {:?} on {} ({:?}).",
            info.backend, info.name, info.device_type
        );
        return;
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
        println!("{} version: {}, renderer: {}.", api, version, renderer);
        return;
    }
    println!("Renderer: unknown backend.");
}

#[cfg(test)]
#[path = "startup_test.rs"]
mod tests;
