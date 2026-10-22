# Build CoCoVM from source

These instructions build the desktop application on macOS, Debian or Ubuntu
Linux, and Windows. Complete the setup for your platform before running Cargo.

## Set up macOS

Install the Xcode Command Line Tools, which provide the C compiler and linker:

```sh
xcode-select --install
```

Complete the installer, then install Rust with [rustup](https://rust-lang.org/tools/install/).
Homebrew and the full Xcode application aren't required to build CoCoVM.

## Set up Debian or Ubuntu Linux

Install the compiler, `pkg-config`, and the development libraries used by the
audio, gamepad, and windowing dependencies:

```sh
sudo apt-get update
sudo apt-get install git build-essential pkg-config \
  libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
```

Then install Rust with [rustup](https://rust-lang.org/tools/install/). The
`libudev-dev` package provides `libudev.pc`, which `pkg-config` needs when
building the gamepad dependency.

## Set up Windows

Install [Git for Windows](https://git-scm.com/download/win). Install
[Microsoft C++ Build Tools](https://visualstudio.microsoft.com/downloads/)
and select the **Desktop development with C++** workload. It includes the
MSVC linker and Windows SDK needed by Rust. An existing Visual Studio
installation with that workload also works.

Then install Rust with [rustup](https://rust-lang.org/tools/install/). Use
the default MSVC toolchain, and open a new PowerShell window after installation
so `cargo` is on your `PATH`.

## Build and run

In a terminal on macOS or Linux, or PowerShell on Windows, clone the repository:

```sh
git clone https://github.com/sperano/cocovm.git
cd cocovm
```

To build and launch CoCoVM during development, run:

```sh
cargo run
```

To create an optimized executable, run:

```sh
cargo build --release
```

The executable is `target/release/cocovm` on macOS and Linux, or
`target/release/cocovm.exe` on Windows. On macOS, the executable is not an
`.app` bundle; see [packaging/README.md](packaging/README.md) to create one.
On first launch, CoCoVM prompts to download the required runtime assets.
