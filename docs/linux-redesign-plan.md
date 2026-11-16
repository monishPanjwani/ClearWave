# ClearWave Linux redesign execution plan

## Execution result — 2026-11-16

Implemented by a delegated `gpt-6.1-sol` workhorse and reviewed by the parent agent. Linux input/output sessions, independent background engine, three-page UI, live DSP controls, settings/custom presets, and Linux-only packaging are implemented. The chosen DSP is original Rust based on documented equations, not a translation of FxSound. See `THIRD_PARTY_NOTICES.md` for inspected upstream files and differences, and `docs/validation.md` for checks and remaining manual tests.

Formatting, strict Clippy, 29 tests with and without GUI features, and the release build passed. Generated stereo signals were verified on an isolated real PipeWire server; native GUI close/reopen and cleanup were verified against generated-device fixtures. Parent review additionally inspected rendered UI, checked the diff, and reran formatting, Clippy and the 29 default-feature tests. Physical hardware/listening tests and AppImage assembly remain unverified. Changes are local; no commit, push, release or service installation was performed.

## Agreed product scope

Linux-only Rust desktop application, inspired by FxSound playback enhancement, with equally useful microphone processing. Separate Output, Input, and Advanced pages. Minimal, polished, responsive UI. Audio must continue independently of the visible window when background operation is selected. User intends personal use and sharing with friends; no commercial launch is planned. Sharing still requires observing upstream license terms.

Additional requirement: follow the active Omarchy color theme automatically, with an in-app menu offering the same Omarchy palettes on other Linux distributions. This theme work is **implemented as of 2026-11-17**. Antigravity supplied the palette/module draft; Codex completed UI integration, corrected parsing/live reload, added provenance and validation, and rebuilt the binary. See `docs/antigravity-results.md` for evidence and remaining manual checks.

## Source investigation and licensing

Reference repository: https://github.com/fxsound2/fxsound-app, inspected main commit `d6d5b740f3efd29e5afcef6724bfd28f2827bc3b`.

Read `dsp/DfxDspEq.cpp` and `dsp/include/DfxDsp.h`, then inspect the actual `dsp/ptutil/DspUtil/GraphicEq/` coefficient/processing implementation and relevant effects as needed. The current upstream offers variable EQ bands, band frequency/gain, normalization, volume leveling, master gain, Q, and Fidelity/Ambience/Surround/DynamicBoost/Bass effects. It is not evidence of a universal optimal EQ. Windows registry synchronization, drivers, and JUCE are unsuitable foundations for the Linux backend.

Do not claim bit-identical FxSound behavior without comparison tests. Implement the requested feature set in Rust, using documented DSP equations where appropriate. For any translated/adapted upstream implementation, preserve its copyright notices, mark modifications, include AGPL-3.0 text, license the covered application compatibly, and record exact upstream files/commit in THIRD_PARTY_NOTICES.md. Inspect individual licenses before using assets. Use original ClearWave branding and original presets. Do not simply retain MIT metadata over an AGPL port. Document which implementation is original and which is derived; reading code alone is not proof of derivation.

## Architecture and behavior

1. Preserve the existing useful Linux CLI contracts and microphone cleanup tests. Remove Windows backend, target dependencies, PowerShell builder, Windows CI/release jobs, and obsolete documentation. Support Linux only explicitly.
2. Build separately startable/stoppable input and output processing sessions using PipeWire's PulseAudio compatibility (and PulseAudio if the implementation supports it). Reuse existing safe process/module ownership and cleanup. Output: application streams -> ClearWave virtual stereo sink -> Rust DSP -> selected physical speakers/headphones. Input: physical source -> Rust DSP -> ClearWave virtual microphone. Exclude own virtual endpoints from physical-device selection and prevent feedback routes.
3. Output DSP: meaningful multiband EQ, gain/attenuation, bass/clarity, stereo width, loudness/compression, optional ambience if implemented properly, and a limiter. Original useful presets (Neutral, Music, Movies, Voice). Avoid cosmetic controls with no DSP behind them. Keep stereo independent and use linked limiting to preserve image.
4. Input DSP: retain RNNoise, add low-cut/rumble reduction, EQ, compressor, attenuation as well as boost, limiter, and original Natural/Clear Speech/Warm Voice/Noisy Room presets. Monitoring is off by default and explicitly selects headphones. Explain physical input clipping cannot be repaired. Distinguish microphone hardware volume from processed gain.
5. Smooth gain/effect changes and transition filter updates without loud discontinuities or unstable state. Validate all finite/range values at the settings boundary. Avoid allocations, subprocess calls, disk IO, and long locks in DSP sample loops. Real controls must update during processing; device changes may safely restart only their affected pipeline.
6. Separate engine lifetime from GUI via a per-user daemon and local IPC (or a comparably robust implementation). Single engine per user, private socket/runtime directory, bounded request size/timeouts, no remotely exposed port. UI is asynchronous and remains responsive during slow/missing audio-server operations. Closing the window with background enabled preserves audio; reopening attaches to the same engine. Explicit Quit/Stop background stops both sessions and cleans owned children/modules. Foreground CLI remains available. Idle engine should not poll/render at high frequency. No hidden recording or persistent audio storage.
7. Persist versioned settings/presets atomically under an appropriate XDG application location, validate on load, handle malformed data visibly and safely. Never silently start capturing just from opening a window. Background-at-login should be opt-in; provide packaging/documentation without installing/enabling a user service on this machine. Existing default device changes should restore only if ClearWave still owns the default; do not override later user choices. Document existing app streams may need explicit selection/restart. Recover or report unplug/server failure and permit retry without hanging.

## Interface

- Original understated design, readable typography, generous spacing, consistent alignment and clear disabled/error states. Replace fixed mint/dark colors with semantic palette tokens that support Omarchy light and dark themes. Responsive at the documented minimum window size; scrolling where needed.
- Output: physical destination, preset, enable/stop, processed output level/gain, concise EQ and basic effects. Input: source, preset, enable/stop, noise cleanup, gain, EQ, input/output meters.
- Advanced: detailed DSP parameters, monitoring, default-device/routing controls, background behavior, diagnostics and explicit shutdown. Keep implementation details out of ordinary user flows.
- Smooth meter decay and control interactions; limited repaint cadence while active and no perpetual high-rate idle repaint. Clearly show both pipelines' states and errors even when their page is hidden. Preset edits become Custom. Include reset-to-neutral.

## Next phase: Omarchy themes and portable theme menu

### User experience

- Add a compact theme menu in the window header, reachable from Output, Input and Advanced. Put any detailed appearance settings in Advanced. Show palette names and small color swatches; preserve the existing minimal layout.
- Offer **Follow Omarchy**, named bundled Omarchy palettes, and **ClearWave Classic** (the current dark/mint palette) as a fallback. On first launch, follow Omarchy when a valid active palette exists; otherwise use a bundled default. Show an explanation when Follow Omarchy is selected on a machine without Omarchy.
- Manual selection applies to ClearWave only and overrides automatic following until the user selects Follow Omarchy again. Choosing a palette must never change the desktop's theme.
- Bundle palette data so the same choices work offline on other Linux distributions without Omarchy, Hyprland, its CLI, or a network download. Include both light and dark palettes where provided by the selected upstream theme set. Record the shipped theme list and upstream revision instead of promising every future community theme.
- Apply changes live across every page, menu, slider, meter, focus ring, tooltip, disabled state, and error message. Theme switching preserves audio, page selection, unsaved control edits and presets. Use a short restrained color transition; avoid flashing, layout shifts, or repeated restarts.

### Theme sources and implementation

1. Add a dedicated theme module with semantic tokens (background, surface, text, secondary text, accent, selection, border, success, warning, error and meter colors). Remove hard-coded UI colors in favor of those tokens. Audio state must remain understandable through labels/icons as well as color.
2. Read Omarchy's active palette as data only. Inspection of this machine on 2026-11-16 found the installed Omarchy scripts using `~/.local/state/omarchy/current/theme/colors.toml` and `theme.name`; `~/.config/omarchy/current/theme/colors.toml` was absent. Honor absolute `XDG_STATE_HOME` when locating state, use the home-directory fallback, and support the older config path as a compatibility fallback. Verify against the targeted Omarchy version before implementing rather than assuming one layout is universal.
3. Parse `colors.toml` safely, using keys such as `mode`, `background`, `lighter_background`, `foreground`, `accent`, `selection`, `muted`, and semantic red/yellow/green colors. Tokyo Night's installed file confirms these fields; other palettes may omit fields, so derive missing tokens with documented defaults. Validate color values and bound file size. Treat files as data; never execute theme scripts or source shell configuration.
4. Observe the parent directory/state location as well as the palette file: Omarchy can replace the entire `current/theme` directory during theme changes. Debounce transient missing/partial files, reload without a GUI restart, retain the last valid palette during replacement, and use a readable fallback with a nonblocking explanation on persistent failure. Avoid busy polling and disk reads on every frame. Reopening the GUI reads the current theme; a headless audio daemon needs no theme watcher.
5. Bundle named palette snapshots in an application assets directory. Confirm the license and provenance for each palette before copying it, retain required notices, and include the data/notices in the binary or package so AppImage and standalone use work on other distros. Do not copy Omarchy backgrounds, logos, fonts, or whole theme directories just to obtain colors. Document any derived colors separately from upstream values.
6. Persist the user's choice as an optional/backward-compatible appearance setting, with a stable theme identifier and safe behavior for unknown/removed themes. Keep appearance changes separate from audio reconfiguration: changing a theme must not restart sessions, alter routing/defaults, or enable microphone capture. Existing version-1 settings must continue to load.
7. Respect the palette's light/dark mode, and verify readable text, button labels, selections and meters on both. Use contrast checks for ordinary text and essential controls; if an upstream secondary color is too faint, derive an accessible UI token without silently replacing the theme's core colors. Missing fonts should fall back cleanly to the existing bundled UI font.

### Execution and acceptance criteria

1. Inspect the current UI/settings and installed Omarchy theme format, choose and record the initial bundled palette set with licenses, then implement theme tokens and the theme source adapter.
2. Add the global menu, persistence and live following. Keep this work scoped to repository files; no desktop config edits, theme hooks, service installation or system theme changes are required.
3. Test automatic detection, absolute XDG paths and home fallbacks, manual override, persisted selection, older settings migration, invalid/oversized/missing palettes, missing fields, and whole-directory replacement. Use temporary fixture palettes rather than modifying the user's active theme.
4. Confirm every bundled theme works when Omarchy directories and commands are absent. Verify rapid theme changes leave both audio session IDs unchanged and never activate stopped input/output sessions. Confirm idle behavior does not introduce high-rate redraw or polling.
5. Capture and inspect Output, Input and Advanced in at least one light and two dark palettes, including the minimum supported window size and the theme menu. Test keyboard navigation, focus visibility, long theme names, selected text, warnings and disabled controls.
6. Run formatting, strict Clippy, both test configurations and release build. Update README, theme provenance/notices, screenshots and validation notes with verified behavior and any remaining limitations. Report this phase as complete only after implementation and verification.

## Execution sequence and acceptance

1. Inspect existing code and upstream, write concise design/provenance notes.
2. Implement validated settings and DSP modules with behavior tests.
3. Implement routing sessions, background lifecycle/IPC, persistence, and CLI integration.
4. Replace UI with the three-page design and live settings.
5. Update Linux packaging, source-build instructions, troubleshooting, licenses/notices, and checks.
6. Run `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked`, `cargo test --locked --no-default-features`, and release build. Validate packaging scripts.
7. Add meaningful tests: neutral EQ response, EQ frequency response, stereo isolation, finite bounded samples, attenuation/boost/compression/limiting, live setting transitions, malformed settings, IPC timeout/single-instance/shutdown, and owned-routing cleanup/default restoration/failure rollback with mocks where possible.
8. Exercise real routing only without capturing the user's microphone or interrupting existing audio: isolated/null-sink generated signals where supported. Report which real hardware/listening checks remain. Capture a GUI screenshot if a safe available mechanism exists; do not claim visual verification without viewing it. No browser server or browser preview for this desktop task.

Do not publish a new release or change installed system configuration during implementation. Leave changes reviewable; report exact checks, limitations, and remaining defects. Do not promise zero mistakes. A complete working Linux application takes priority over elaborate effects with unverified behavior.
