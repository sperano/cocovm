use std::path::PathBuf;

use super::*;

#[test]
fn unpack_assets_extracts_gzipped_tar_into_dest() {
    let dest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp-test-assets/unpack");
    let _ = std::fs::remove_dir_all(&dest);

    // Build a cocovm-assets-shaped tarball in memory: roms/ and images/.
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tarball = tar::Builder::new(gz);
    for (path, contents) in [
        ("roms/test.rom", &b"\xAA\xBB"[..]),
        ("images/blank.dsk", &b"\x00\x01"[..]),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tarball.append_data(&mut header, path, contents).unwrap();
    }
    let bytes = tarball.into_inner().unwrap().finish().unwrap();

    unpack_assets(&bytes[..], &dest).unwrap();
    assert_eq!(std::fs::read(dest.join("roms/test.rom")).unwrap(), b"\xAA\xBB");
    assert_eq!(std::fs::read(dest.join("images/blank.dsk")).unwrap(), b"\x00\x01");
}
