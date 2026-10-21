use crate::machine_def::tests::full_def;

/// Every path-bearing field is listed, in field order; empty and unused
/// MPI slots add nothing.
#[test]
fn host_paths_lists_every_path_field() {
    let def = full_def();

    assert_eq!(
        def.host_paths(),
        [
            "dev.dsk",
            "/shared/utils.dsk",
            "/vhd/68SDC.VHD",
            "session.cas",
            "dw0.dsk",
            "/shared/dw2.dsk",
            "/shared/games",
            "prints",
            "/paks/arkanoid.ccc",
            "/path/custom.rom",
        ]
    );
}

/// A relative system ROM resolves against the working directory, never the
/// artifact dir, so it isn't listed; blank paths aren't either.
#[test]
fn host_paths_skips_relative_rom_and_blank_paths() {
    let mut def = full_def();
    def.hardware.rom = Some("custom.rom".to_string());
    def.drivewire.printer.folder = " ".to_string();

    let paths = def.host_paths();

    assert!(!paths.contains(&"custom.rom"), "{paths:?}");
    assert!(!paths.contains(&" "), "{paths:?}");
}
