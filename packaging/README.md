# Packaging

Everything that gives the `cocovm` binary an application icon outside the
running window. The release workflow (`.github/workflows/release.yml`) calls
these scripts; they also work on a local `cargo build --release`.

| Platform | Mechanism | Files |
|---|---|---|
| macOS | `cocovm.app` bundle with `Info.plist` and `cocovm.icns` | `macos/bundle.sh BINARY VERSION APP_DIR` |
| Linux | `.desktop` entry plus hicolor icons, installed by `install.sh` | `linux/cocovm.desktop`, `linux/install.sh`, `linux/stage.sh BINARY STAGE_DIR` |
| Windows | Icon and version block compiled into the `.exe` | `crates/coco-egui/build.rs`, `crates/coco-egui/assets/cocovm.ico` |

`make-icons.sh` uses ImageMagick 7 to generate `icons/hicolor/`, the Windows
`.ico`, and `crates/coco-egui/assets/cocovm-icon-macos.png` from the approved
artwork in `crates/coco-egui/assets/cocovm-icon.png`. Run it after changing the
artwork and commit the generated files.

The macOS export scales the full illustration into an 824-pixel rounded square
on a transparent 1024-pixel canvas, with 100 pixels of padding on each side.
Its antialiased mask uses a 185-pixel corner radius. To regenerate only this
export, run `packaging/make-icons.sh --macos-only`.

The macOS runtime embeds this export for the Dock icon. Both the local release
script and the release workflow use `macos/bundle.sh` to generate the Finder
icon from the same export, so the Finder and Dock icons match. The bundle
script uses macOS `sips` and `iconutil`; release builds don't need ImageMagick.

Local macOS bundle:

```sh
cargo build --release -p coco-egui
packaging/macos/bundle.sh target/release/cocovm 0.0.0 build/cocovm.app
```
