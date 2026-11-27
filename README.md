# AllOS Emulator

AllOS Emulator is a lightweight Tauri desktop front end for managing local QEMU virtual machines. It supports configuration presets for `x86_64`, `aarch64`, and `riscv64`, process start/stop/pause/resume/restart, live stdout/stderr display, and heuristic log diagnostics.

## Important scope and prerequisites

This application is a **QEMU manager**, not a bundled copy of QEMU and not a universal compatibility layer. QEMU executables, firmware files, guest operating-system images, and any required host virtualization features must be installed and licensed separately. The desktop application itself does not include Windows, macOS, Android, Linux, OpenCore, EDK2 firmware, or QEMU.

Guest support depends on the host operating system, QEMU build, guest architecture, firmware, and applicable vendor licensing. In particular, macOS virtualization on non-Apple hardware is not enabled or endorsed by this project. A filename-based preset is only a suggestion; it cannot reliably identify the actual guest OS or boot mode.

The current backend validates a selected image and QEMU executable, launches QEMU, reads stdout/stderr, and exposes process controls. The generic drive argument currently treats the selected image as a qcow2 VirtIO drive; ISO boot media and other disk formats may require adjusting the QEMU arguments for the particular guest. UEFI firmware path is also host-specific and must be configured for the installed QEMU environment. The UI's screenshot control is informational only; it does not capture a framebuffer.

## Requirements

- Rust stable toolchain and Cargo.
- Node.js is not required for this static HTML/CSS/JavaScript frontend.
- Tauri v2 system prerequisites:
  - Windows: Microsoft C++ Build Tools and WebView2 Runtime.
  - macOS: Xcode Command Line Tools.
  - Ubuntu/Debian: `webkit2gtk-4.1`, `build-essential`, `curl`, `wget`, `file`, `libxdo-dev`, `libssl-dev`, `librsvg2-dev`, `libayatana-appindicator3-dev`, `libgtk-3-dev`.
- A QEMU system emulator binary matching the VM architecture.

## Development

From the repository root:

```sh
cargo tauri dev
```

If the Tauri CLI is not installed:

```sh
cargo install tauri-cli --version "^2"
cargo tauri dev
```

Build:

```sh
cargo tauri build
```

The frontend can also be previewed as static files, but QEMU process control is available only inside the Tauri desktop runtime.

## QEMU executable examples

- Linux x86_64: `/usr/bin/qemu-system-x86_64`
- Linux AArch64: `/usr/bin/qemu-system-aarch64`
- Linux RISC-V: `/usr/bin/qemu-system-riscv64`
- Windows: point to the installed `qemu-system-*.exe` binary.
- macOS: point to the QEMU executable installed by your package manager.

## VM configuration notes

The automatic preset uses filename hints such as `windows`, `android`, `arm64`, and `riscv`. It does not inspect guest boot metadata. Verify memory, CPU, architecture, firmware, disk format, graphics and network settings before starting a guest.

Process pause/resume uses operating-system process signals on Unix and PowerShell process suspension on Windows. These controls may be restricted by host permissions and are not equivalent to QEMU's guest-level QMP pause command. Stop terminates the QEMU process.

## GitHub Actions

The workflow in `.github/workflows/build.yml` builds Windows, macOS, and Ubuntu packages. A push to `main` or a version tag triggers builds. On pushes to `main`, artifacts are published to a rolling `nightly` prerelease; version tags publish a release matching the tag. The repository's `GITHUB_TOKEN` is used by the release action.

## Security

QEMU is launched directly without a shell. Extra arguments are split on whitespace and passed as individual process arguments. Do not paste untrusted command-line content or secrets into the extra-arguments field. VM images are opened by the local QEMU installation and are not uploaded by this application.

## License

This repository's application code is provided under the MIT License. Third-party components retain their own licenses.
