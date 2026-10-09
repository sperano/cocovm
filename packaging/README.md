# Packaging

Everything that gives the `cocovm` binary an application icon outside the
running window. The release workflow (`.github/workflows/release.yml`) calls
these scripts; they also work on a local `cargo build --release`.

| Platform | Mechanism | Files |
|---|---|---|
| macOS | `cocovm.app` bundle with `Info.plist` and `cocovm.icns` | `macos/bundle.sh BINARY VERSION APP_DIR` |
| Linux | `.desktop` entry plus hicolor icons, installed by `install.sh` | `linux/cocovm.desktop`, `linux/install.sh`, `linux/stage.sh BINARY STAGE_DIR` |
| Windows | Icon and version block compiled into the `.exe` | `crates/coco-egui/build.rs`, `crates/coco-egui/assets/cocovm.ico` |

`icons/hicolor/` and the `.ico` are generated from
`crates/coco-egui/assets/cocovm-icon.png` by `make-icons.sh` (ImageMagick 7).
Re-run it after changing the artwork and commit the result.

Local macOS bundle:

```sh
cargo build --release -p coco-egui
packaging/macos/bundle.sh target/release/cocovm 0.0.0 build/cocovm.app
```
