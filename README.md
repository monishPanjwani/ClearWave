# Clearwave

A small, local Rust desktop app for microphone cleanup and audio routing, with a Linux CLI for automation. No accounts, cloud processing, audio uploads, or recordings. The desktop interface uses egui; the noise reducer uses RNNoise through `nnnoiseless`.

**Status: Linux release builds locally; automated DSP/backend/CLI checks are available.** Live noise quality still needs listening checks with your microphone. Windows remains experimental and has not been tested on Windows.

## What it does

- Select and inspect the microphone and output device.
- Suppress background noise locally, silence residual noise between phrases with a speech-aware gate, then apply up to 18 dB of microphone boost with a peak limiter. The gate is included in “Remove background noise”; it adds 10 ms of lookahead, holds word endings for 200 ms, and fades closed to avoid clicks. Very quiet speech may still be misclassified; disable cleanup to compare.
- Display live input/output levels and input clipping information while processing.
- Linux: create a temporary **Clearwave Microphone** for calls, games, and recording apps; adjust system device volume and defaults; optionally monitor through headphones.
- Windows: process microphone audio into an existing playback device, such as a virtual cable. See the Windows limitations below.

Processing starts only when you press Start or invoke `run`. Use headphones for monitoring to avoid feedback. Noise reduction is tuned for speech; it cannot remove every sound, repair clipped input, or perform acoustic echo cancellation. If input clips, reduce the microphone's hardware/system gain before adding software boost.

## Copy, build, and run from source

Install Git, stable [Rust](https://rustup.rs/), and the platform dependencies listed below. Then clone the project:

```sh
git clone https://github.com/monishPanjwani/ClearWave.git
cd ClearWave
```

On Linux:

```sh
./scripts/build.sh
./dist/clearwave
```

On Windows, open PowerShell after installing Visual Studio C++ Build Tools:

```powershell
.\scripts\build.ps1
.\dist\clearwave.exe
```

## Download or build release packages

Download `clearwave.exe` or `Clearwave-x86_64.AppImage` from [GitHub Releases](https://github.com/monishPanjwani/ClearWave/releases). The EXE runs directly on Windows. On Linux:

```sh
chmod +x Clearwave-x86_64.AppImage
./Clearwave-x86_64.AppImage
```

The AppImage targets x86_64 Linux with glibc 2.35 or newer. It still requires the audio utilities and desktop libraries described below. If FUSE is unavailable, run it with `--appimage-extract-and-run`.

To make your own packages, run `./scripts/build-appimage.sh` on x86_64 Linux (requires `curl` and downloads appimagetool), or `.\scripts\build.ps1` on Windows. Outputs go into `dist/`. Push a version tag such as `v0.1.0` to run the GitHub release workflow, which tests and builds both platforms and publishes the binaries with SHA-256 checksums.

## Build and launch on Linux

Install a current stable [Rust toolchain](https://rustup.rs/) and a C compiler. GUI builds need the platform's Wayland/X11/OpenGL development libraries. For example, Debian/Ubuntu needs `build-essential pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev`; distro package names may differ.

At runtime, run under your normal desktop user with **PipeWire plus pipewire-pulse**, or **PulseAudio**. `pactl`, `parec`, and `pacat` must be on PATH (usually `pulseaudio-utils` on Debian/Ubuntu or `libpulse` on Arch). `pactl` must support JSON output. No root permissions are needed for audio routing.

```sh
./scripts/build.sh
./dist/clearwave doctor --json
./dist/clearwave
```

To build and install a launcher entry under your user account:

```sh
./scripts/install-desktop.sh
```

This builds first, copies the executable to `~/.local/share/clearwave/clearwave`, and installs `~/.local/share/applications/clearwave.desktop` (or the corresponding directories under `XDG_DATA_HOME`). Then search for **Clearwave** in the app launcher. The build script automatically finds Rust in `~/.cargo/bin` if it is missing from PATH. The first build needs internet access to download Rust dependencies.

For a smaller Linux CLI executable without the desktop dependencies:

```sh
./scripts/build.sh cli
./dist/clearwave --help
```

The executable still requires the operating system's libraries and audio utilities; it is not a fully static, universal Linux binary. The `cli` build replaces the executable in `dist/`.

## Use the cleaned microphone

1. Select a physical microphone, keep noise cleanup enabled, and start with 0 dB boost.
2. Press Start. Choose **Clearwave Microphone** in your calling/recording application's microphone settings.
3. Speak and check the input and cleaned-output meters. Add boost only if needed.
4. Open the routing inspector to see exact source/destination IDs. Stop before changing processing settings.
5. Keep Clearwave running during your call. Stopping removes its temporary audio devices.

The speaker selector identifies the Linux monitoring destination; changing the system default is a separate, explicit action. Existing application streams may keep their old device until reselected or restarted. Headphone monitoring is optional and off by default.

## Linux CLI for agents

```sh
clearwave doctor --json
clearwave devices --json
clearwave run --input 'SOURCE_ID' --boost-db 6 --make-default --json
clearwave run --input 'SOURCE_ID' --output 'HEADPHONE_SINK_ID' --monitor
clearwave volume output 'SINK_ID' 80 --json
clearwave default output 'SINK_ID' --json
```

Use IDs from `devices`, not display names. `doctor` and `devices` do not capture audio. `run` stays in the foreground; supervise it as a normal process and send SIGINT or SIGTERM to stop it. JSON mode emits newline-delimited objects: `started` includes the virtual source ID, `levels` reports measured state, and `stopped` confirms normal shutdown. Errors go to stderr as JSON; exit status is nonzero. CLI argument errors use exit code 2.

`--make-default` temporarily switches the default microphone and restores it on normal shutdown only if it still points to this session. `default` and `volume` are explicit system changes and persist after the command. No automatic package installation, startup service, or background daemon is installed. Abrupt termination with SIGKILL or a crash cannot guarantee module/default cleanup; inspect `pactl list short modules` and remove only the affected Clearwave module IDs with `pactl unload-module ID`.

## Windows desktop

Install stable Rust with the MSVC toolchain and Visual Studio C++ Build Tools, then:

```powershell
.\scripts\build.ps1
.\dist\clearwave.exe
```

The Windows backend is experimental and has not been compiled or tested on Windows. Enable microphone access for desktop apps in Windows settings. Select your physical input and an explicit output destination. To use cleanup in another application, install a virtual audio cable separately, select its **playback** endpoint as Clearwave's output, and select its paired **recording** endpoint as the other application's microphone. Clearwave does not install a Windows audio driver or automatically create a Windows virtual microphone. Selecting real speakers plays the processed voice aloud and can cause feedback.

Windows system volume/default changes are not implemented in this version; use Windows Sound settings. The automation CLI is Linux-only. A Windows build must be built on Windows or with a separately configured cross-compilation toolchain.

## Compatibility boundaries

| Environment | Current implementation |
| --- | --- |
| PipeWire with PulseAudio compatibility | Linux device discovery, virtual mic, cleanup, routing, volume |
| PulseAudio | Same command-based backend; JSON-capable `pactl` required |
| ALSA-only / JACK-only | Not implemented; install/configure a supported desktop audio server |
| USB / built-in / Bluetooth microphones | Usable when exposed as capture devices by the supported audio server; availability and quality depend on the hardware/profile |
| Windows WASAPI | Experimental capture/playback backend; separate virtual cable needed for other apps |

No application can promise support for every Linux driver or microphone. Clearwave uses devices already working in the operating system. Bluetooth headset profiles, permissions, unplugged devices, and audio-server failures are reported rather than silently simulated. Reconnect and refresh/restart after device changes. Automatic reconnection, saved profiles, autostart, echo cancellation, and a bundled Windows virtual driver are future work.

## Development

```sh
cargo build
cargo run -- doctor --json
cargo run
cargo test
cargo test --no-default-features
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo build --release
```

`Cargo.lock` is now present from the local build; preserve it when distributing this executable and use `--locked`. Dependencies cached by that build allow offline rebuilds. Rustfmt has been applied; the Linux build script and CI check formatting with `cargo fmt --check`. No CI run has been performed here.

`src/dsp.rs` owns 48 kHz mono cleanup/boost/limiting. `src/platform/` owns OS devices and session lifecycles. `src/ui.rs` owns the desktop interface and its background command worker. `src/main.rs` owns Linux CLI orchestration. `tests/` contains CLI contract tests; module tests cover processing and backend parsing. Format with rustfmt and lint with Clippy. Audio buffers are memory-only.

Before release, test with real USB, built-in, and Bluetooth microphones on both systems; verify virtual-device selection in a calling app, clipping behavior, latency, CPU use, unplug/replug, audio-server restart, and cleanup on stop. Unit tests do not establish audio quality or driver compatibility.

Implementation references: [RNNoise frame/sample format](https://docs.rs/nnnoiseless/0.5.2/nnnoiseless/struct.DenoiseState.html), [PipeWire null sink](https://docs.pipewire.org/page_pulse_module_null_sink.html), [PipeWire remap source](https://docs.pipewire.org/page_pulse_module_remap_source.html), and [CPAL stream APIs](https://docs.rs/cpal/0.15.3/cpal/traits/trait.DeviceTrait.html).
