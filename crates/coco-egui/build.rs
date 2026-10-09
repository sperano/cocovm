//! Embeds the application icon and version resource into the Windows
//! executable when building on a Windows host (cfg and the `winresource`
//! build dependency are host-side). A cross build from another host
//! ships no icon; the release workflow builds Windows on Windows.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    windows_resources();
}

#[cfg(windows)]
fn windows_resources() {
    // Regenerate with packaging/make-icons.sh after changing the artwork.
    const ICON: &str = "assets/cocovm.ico";
    println!("cargo:rerun-if-changed={ICON}");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(ICON)
        .set("ProductName", "CoCoVM")
        .set("FileDescription", "CoCoVM Tandy Color Computer emulator")
        .set("OriginalFilename", "cocovm.exe")
        .set("LegalCopyright", env!("CARGO_PKG_LICENSE"));
    resource
        .compile()
        .unwrap_or_else(|error| panic!("embedding Windows resources failed: {error}"));
}
