//! Guards the `cargo package` file list for the tms7000 crate:
//! the MAME BSD-3-Clause NOTICE must ship, and no packaged test or example may
//! use the workspace-only `cocovm-test-assets` dev-dependency (packaging strips
//! path-only dev-dependencies without a version, so such a target would fail to
//! build from the published crate). This file itself is excluded from the
//! package, so it can't check itself into a corner.

use std::path::Path;
use std::process::Command;

#[test]
fn cargo_package_list_has_notice_but_not_workspace_only_targets() {
    let output = Command::new(env!("CARGO"))
        .args([
            "package",
            "-p",
            "tms7000",
            "--allow-dirty",
            "--no-verify",
            "--offline",
            "--list",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo package --list");
    assert!(
        output.status.success(),
        "cargo package --list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let listing = String::from_utf8_lossy(&output.stdout);
    let files: Vec<&str> = listing.lines().collect();

    assert!(
        files.contains(&"NOTICE"),
        "packaged crate must ship the MAME BSD-3-Clause NOTICE: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.ends_with("firmware_boot.rs")),
        "firmware_boot.rs (needs cocovm-test-assets) must be excluded: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.ends_with("firmware_trace.rs")),
        "firmware_trace.rs (needs cocovm-test-assets) must be excluded: {files:?}"
    );

    // General invariant, not just the two known offenders above: nothing
    // packaged under tests/ or examples/ may reference the workspace-only
    // cocovm-test-assets dev-dependency, since packaging strips it.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in files
        .iter()
        .filter(|f| f.starts_with("tests/") || f.starts_with("examples/"))
    {
        let source = std::fs::read_to_string(manifest_dir.join(file))
            .unwrap_or_else(|e| panic!("read packaged file {file}: {e}"));
        assert!(
            !source.contains("test_assets"),
            "{file} is packaged but needs the workspace-only cocovm-test-assets; \
             add it to package.exclude"
        );
    }
}
