use super::*;

/// Encode a tiny valid PNG the `image` crate can round-trip.
fn write_test_png(path: &Path, w: u32, h: u32) {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    img.save(path).unwrap();
}

#[test]
fn random_from_dir_picks_and_decodes_an_image() {
    let dir = std::env::temp_dir().join("coco-photo-view-test-picks");
    std::fs::create_dir_all(&dir).unwrap();
    write_test_png(&dir.join("page-1.png"), 4, 6);
    std::fs::write(dir.join("not-an-image.txt"), b"ignored").unwrap();
    // AppleDouble sidecar: right extension, but hidden and not a PNG.
    std::fs::write(dir.join("._page-1.png"), b"AppleDouble junk").unwrap();

    let photo = random_from_dir(&dir).expect("an image exists, so a photo is decoded");
    assert_eq!(photo.title, "page-1");
    assert_eq!(photo.pixels.size, [4, 6]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn random_from_dir_yields_none_when_no_images() {
    let dir = std::env::temp_dir().join("coco-photo-view-test-empty");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(random_from_dir(&dir).is_none());
    std::fs::remove_dir_all(&dir).unwrap();

    assert!(random_from_dir(Path::new("/nonexistent-dir")).is_none());
}
