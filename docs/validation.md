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
- AppImage assembly/download and desktop/service installation were not executed;
  their scripts/templates were checked without changing installed configuration.
- The 520×620 logical minimum uses scrolling by design, but this exact minimum
  was not separately captured. Screenshots show the default window size.
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
