# Linux redesign validation

Validated on this Linux workspace on 2026-11-16. No user microphone was recorded,
no real audio default was changed, no service was installed/enabled, and nothing
was committed, pushed or released.

## Automated checks

- `cargo fmt --check`
- `cargo clippy --locked --all-targets -- -D warnings`
- `cargo test --locked`: 25 unit tests, 3 CLI tests, 1 process lifecycle test.
- `cargo test --locked --no-default-features`: the same 29 tests.
- `cargo build --release --locked`
- `bash -n scripts/build.sh scripts/build-appimage.sh scripts/install-desktop.sh`
- Python AST syntax checks for the license collector and isolated-audio script.
- `desktop-file-validate packaging/clearwave.desktop`
- `systemd-analyze --user verify packaging/clearwave.service` (verification only).
- `scripts/collect-licenses.py`: 182 dependencies in the default Linux dependency
  graph, with notice files for every entry after supplementing upstream notices
  omitted from published crate archives. The bundle is generated under ignored
  `dist/licenses/`, alongside package/version/license/source metadata.

DSP tests verify neutral response, center-frequency boost and off-band response,
stereo isolation, attenuation, boost, linked limiting, compression, finite samples,
rapid repeated EQ-update continuity, gain transitions, RNNoise bypass and the
existing speech gate's onset/hold/release behavior. Invalid settings/presets,
malformed/unknown-version settings, atomic persistence round-trip, message-size
limits, total IPC deadlines (including dribbling bytes), singleton socket ownership,
stale socket replacement, timeout and clean socket removal are covered.

The process-isolated test (`tests/engine_lifecycle.rs`) replaces pactl/parec/pacat
with `tests/fixtures/audio-server.py`. It generates silence rather than opening
an audio device. It verifies both pipelines run concurrently, live configuration,
independent stop/restart, live process route checks, temporary default restoration,
preservation of a later user's default choice, child reaping/module rollback after
playback spawn failure, singleton rejection, persisted settings and explicit shutdown.
The original foreground CLI error/help/JSON tests remain intact.

## Real generated-signal routing check

A temporary private PipeWire 1.6.9 server, private pipewire-pulse and private
WirePlumber were started under a mode-0700 temporary runtime/config directory.
All hardware feature groups and ALSA/Bluetooth/video/MIDI monitors were disabled;
only null sinks and generated data were used. The desktop's server was not used.
Reproduce with `python3 scripts/test-isolated-audio.py` after `cargo build --locked`;
this optional script requires PipeWire, WirePlumber, and PulseAudio client tools.
It does not install or alter services/configuration on the host.

Measured at the private discard sink's stereo monitor:

| Generated 1 kHz test | Left RMS | Right RMS |
| --- | ---: | ---: |
| Neutral, 0.05 peak left-only sine | 0.0355373 | 0 |
| Live processed gain −12 dB | 0.00892709 | 0 |
| Live 1 kHz EQ +6 dB, gain reset to 0 | 0.0709041 | 0 |

The real playback pipeline started, passed generated stereo samples, accepted live
gain/EQ changes, and stopped with no ClearWave modules left in that private server.
An earlier run without session policy timed out visibly and rolled back; a normal
PipeWire desktop requires a session manager/policy that links its streams.

## GUI lifecycle and screenshots

An additional isolated mock-server GUI exercise started both pipelines, enabled
background mode, opened/closed the GUI twice, and verified that both session route
IDs and running states remained unchanged. After switching background mode off,
closing the GUI stopped the daemon and both pipelines, removed the socket and
left no owned modules. These were actual native GUI windows with app-framebuffer
capture, not browser mocks. Only generated silence was processed.

`docs/screenshots/{output,input,advanced}.png` were captured and viewed from a
settled GUI connected to the generated-device fixture. The named generated input
and discard output are test endpoints, not the user's hardware. No desktop pixels
or user microphone audio were captured. The primary Enable/Stop control is above
the processing cards. Main content and Advanced settings scroll vertically.

## Remaining checks and limits

- Physical microphones, speakers/headphones, USB/Bluetooth unplug behavior,
  subjective listening quality and end-to-end hardware latency were not tested.
  No broad hardware-compatibility or FxSound-equivalent claim is made.
- The real-server signal test covers stereo output. Input RNNoise and source routing
  are covered by deterministic DSP/mock tests, not a physical capture session.
- PulseAudio proper was not available for a separate real-server test; its command
  protocol is used and PipeWire-Pulse was exercised.
- Desktop/service installation was not executed. AppImage assembly and smoke
  tests were completed in the 2026-11-18 follow-up below, without changing installed
  configuration.
- The minimum-size view uses scrolling by design. The 520×620 logical light
  appearance was captured during the 2026-11-18 follow-up below.
- SIGKILL/power loss cannot run owned-resource cleanup. Startup detects singleton
  ownership/stale sockets but does not automatically reap modules left by a prior
  forcibly killed process. Normal exit/signals and tested startup failures clean up.
- Audio sample loops do not allocate or call subprocesses/disk I/O. Controls are
  copied with nonblocking try-locks once per frame; input telemetry retains the
  existing short mutex update once per frame. No hard real-time guarantee is made.

## Theme follow-up — 2026-11-17

Theme support was completed after Antigravity's partial implementation. See
[theme handoff results](antigravity-results.md) for exact changes, palette source,
checks, screenshot evidence and remaining manual verification. The final theme
build passes 47 default-feature tests and 30 CLI-only tests. Appearance-only
configuration preserves both active audio routes without updating DSP controls.


## Tray, login startup and interface — 2026-11-18

Implementation: engine-owned ksni 0.3.3 SNI/DBusMenu, bounded menu command queue,
GUI singleton/activation endpoint and explicit shutdown notification. XDG login
startup is reversible and separate from saved default-off processing/default
routing consent. GLib launcher testing found that literal percent signs in argv0
need a shell-free `/usr/bin/env` prefix; startup and desktop installation now
handle that case. Symlink/user-edited conflicts and existing temporary files are
preserved. `APPIMAGE` records the original image path.

Validation performed:

- `cargo fmt --check`, locked strict all-target Clippy, default and headless suites
  passed. Parent review recorded 54 default and 36 headless checks. Added lifecycle
  coverage uses generated mock audio for startup once-only behavior, missing saved
  devices, late server readiness, retry cancellation and explicit default consent.
- `dbus-run-session -- /usr/bin/python3 scripts/test-private-tray.py` passed on
  the desktop binary and packaged AppImage: watcher without host reports unavailable;
  one active service registers when the host appears; DBusMenu toggles login startup
  only in a temporary XDG directory without capture; independent output/input controls,
  watcher restart, StopAll cleanup and Quit/GUI notification all pass.
- `/usr/bin/python3 scripts/test-autostart-exec.py target/release/clearwave` passed
  actual GLib Desktop Entry launch with spaces, quotes, percent signs, dollar signs,
  backticks and backslashes. A moved original AppImage path remains disableable.
- `desktop-file-validate`, shell syntax, Python script parsing and `git diff --check`
  passed. The AppImage's temporary startup entry also passed desktop-file validation.
- `./scripts/build-appimage.sh` built the optimized release and AppImage. All 227
  selected dependency notices are collected, including ksni's UNLICENSE. The image
  includes original PNG/SVG app art and symbolic status icons. Tool/runtime downloads
  are cached and bounded; no installation was performed. Appimagetool reports only
  optional AppStream metadata/tool absence.
- AppImage extraction-mode `--help`, temporary-XDG enable/status/disable and original
  `APPIMAGE` target checks passed. Packaged tray lifecycle passed with generated audio.
  This verifies the local image; it does not establish compatibility across distros.

Parent live review used only laptop `eDP-1`, second workspace `2`; Acer `HDMI-A-1`
remained on workspace `11`. The real Omarchy tray retained the test daemon after
GUI close; independent controls, StopAll and Quit worked with mock audio, including
Quit closing a visible window without respawn. Native close/reopen preserved one
control window. Active dark and light themes plus 520×620 logical size were captured.
System font is discovered through Fontconfig (here JetBrainsMono Nerd Font), with
14 logical px ordinary text and larger headings. Compact wide layout keeps the
main device/meter and EQ/effects controls visible. See [DESIGN.md](../DESIGN.md).

No real startup files, desktop configuration, installed application or live audio
were changed. Physical microphone listening/hardware tests remain outside this
follow-up. Wayland compositor focus policy can still decline foreground focus;
activation uses native GUI focus requests and no compositor-specific commands.

Final review confirmed the packaged binary matches `dist/clearwave` byte-for-byte.
The final image passed the private tray lifecycle test again, and the release
binary passed the GLib autostart test. Test windows and the isolated daemon were
removed; read-only desktop verification found laptop workspace 1 and Acer
workspace 11 afterward.

Updated native screenshots: [Output](screenshots/output.png),
[Input](screenshots/input.png), [Advanced](screenshots/advanced.png). These show
generated test devices and the system's active Ayu Dark palette. Light-theme
minimum-size captures are temporary review artifacts under
`/tmp/clearwave-desktop-review-v6062pw4/captures-light/`.

Final local build SHA-256:

```text
75971e9188b4a8f9a7416f9b821eb058eecc558b4feb657681b1da0e63f9bd79  dist/clearwave
b3ec5ee75955a38cbe6d5192de493cb27d33a7993a2be3c608e3ced8a5321d63  dist/Clearwave-x86_64.AppImage
```
