# Background indicator and login startup

Status: implemented and validated, 2026-11-18. Engine-owned SNI,
reversible XDG startup and explicit saved login policy are implemented. See
[DESIGN.md](../DESIGN.md) for the verified system-font and visual direction.
No real login integration has been enabled and the installed application has not
been updated; builds are in `target/` and `dist/`.

## Goal

Keep ClearWave visible and controllable while its window is closed. Provide a
cross-desktop tray indicator and an in-app login-start setting, including explicit
options to start processing automatically. Preserve independent input/output,
safe cleanup, the Omarchy-aware interface, and existing CLI behavior.

## Research and architecture decision

- Use **StatusNotifierItem (SNI) and DBusMenu on the user's session bus**, rather
  than a custom Omarchy widget or the legacy XEmbed tray. The Rust `ksni` crate
  implements SNI and offers a blocking API suitable for a dedicated control
  thread. Evaluate/pin its current 0.3.x release and dependencies when implementing.
  Sources: https://docs.rs/ksni/latest/ksni/ and https://github.com/iovxw/ksni .
- The installed Omarchy tray imports `Quickshell.Services.SystemTray` in
  `/usr/share/omarchy/shell/plugins/bar/widgets/Tray.qml`. It ignores Passive items
  and may put unpinned items in its drawer. Keep our item Active while the engine
  exists (even when audio is stopped), with idle/processing/error distinguished
  through icon and tooltip. Explain how to pin it; do not edit the user's bar.
  Quickshell reference:
  https://quickshell.org/docs/v0.2.1/types/Quickshell.Services.SystemTray/SystemTray/ .
- Compatibility depends on the desktop's tray host, not its Linux distribution.
  SNI-capable panels should consume the same item; validate actual desktop support
  instead of claiming every distro has a tray. GNOME may need a host extension:
  https://extensions.gnome.org/extension/615/appindicator-support/ .
- Use **XDG desktop autostart** as the portable login integration:
  `$XDG_CONFIG_HOME/autostart/clearwave.desktop`, defaulting to
  `~/.config/autostart/clearwave.desktop`. It runs after graphical login, not before
  login or at machine boot. Reference:
  https://specifications.freedesktop.org/autostart/latest/ .
- UWSM documents XDG autostart integration, but do not assume every Hyprland setup
  consumes desktop autostart files. Verify this Omarchy session's startup path.
  Reference: https://github.com/Vladimir-csp/uwsm . The installed Omarchy startup
  script also invokes `omarchy-hook post-boot`. If XDG autostart is demonstrably
  unavailable, offer an explicit Omarchy adapter using one app-owned post-boot
  hook, with the same launch command and reversible removal. Never enable two
  startup mechanisms or silently edit Hyprland configuration.

## User-visible behavior

Tray tooltip reports actual state, e.g. **ClearWave — Output on · Microphone off**.
Use a recognizable original icon plus separate idle, processing and error states;
do not rely on color alone or animate at meter frequency. Use a symbolic icon and
ARGB fallback pixmaps so it renders even without an installed icon theme.

Menu:

1. **Open ClearWave** — open or raise the existing control window.
2. **Output: On / Off** and **Microphone: On / Off** — read-only state rows.
3. **Start/Stop output** and **Start/Stop microphone** — independent controls.
4. **Stop all processing** — stop both paths, restore owned defaults, retain an
   idle engine/tray for restarting. This is distinct from terminating the app.
5. **Start at login** — checked toggle, synchronized with Advanced settings and
   actual installation state.
6. **Quit ClearWave** — stop both paths, remove virtual devices, release the
   socket/tray and exit. Leave login preference intact for the next login; Quit
   must not trigger automatic respawn in the current session.

Advanced → Background & startup:

- **Keep running when the window closes** (existing option).
- **Start ClearWave when I log in** (new, initially off).
- **Start output processing at login** (explicit subordinate choice).
- **Start microphone processing at login** (separate explicit choice, initially
  off; label that it activates the saved microphone without opening the window).
- **Use processed output/input as system default at login** only if separately
  selected. Starting processing alone does not route existing applications into
  it. Retain ownership-aware default restoration and explain apps that retain a
  specific device may require manual selection.

Enabling login startup does not immediately start capture or restart running audio.
Disabling login startup affects the next login, not current processing. Startup
options are independent of the close-window preference. Reopening the GUI must
never reapply startup policy after the user has stopped processing.

If no tray host exists, keep audio functional, expose the unavailable status in
Advanced, and retain GUI/CLI Stop/Quit. Warn clearly when choosing background mode
without a visible host; do not install shell extensions automatically. Re-register
after tray-host/session-bus restart without restarting audio.

## Implementation sequence

### 1. Engine-owned tray and commands

- Add `src/tray.rs` behind an appropriate desktop feature so `--no-default-features`
  retains a usable headless CLI. Start only after the daemon singleton lock is
  acquired. The tray outlives GUI windows but exits with the daemon.
- Keep D-Bus and menu callbacks off audio threads. Send bounded commands to the
  existing engine control path; never synchronously wait on the engine while
  holding a tray lock. Publish low-rate or change-driven status snapshots.
- Factor stop-all/shutdown into shared handlers used by GUI, CLI and tray. Serialize
  rapid menu actions; show failures rather than optimistic state or swallowed errors.
- Implement GUI activation registration/single-window behavior. Pass Wayland
  activation tokens when available; never depend on Hyprland-specific commands
  to open the production UI. Reject stale GUI registration and permit reopening
  after a window crash. All GUI clients should exit cleanly on explicit Quit.
- Make tray loss nonfatal; distinguish a connected session bus from an actual
  registered tray host. Retry with backoff, not a busy loop. Avoid multiple tray
  icons when autostart and manual launch race.

### 2. Transactional autostart integration

- Add `src/autostart.rs` to inspect, enable and disable app-owned startup files.
  Write atomically with correct Desktop Entry Exec quoting and absolute executable
  paths; no shell evaluation. Test spaces, quotes, percent signs and dollar signs.
- Prefer the stable installed binary. For AppImage, use the original executable
  AppImage path, never its temporary mounted `current_exe()` path. Detect missing
  or moved executables and offer repair through the app. Do not assume a build
  directory will remain present after an update.
- Add an explicit command such as `clearwave startup`. It ensures one daemon,
  applies saved startup policy once per graphical login/startup invocation, and
  exits its launcher without opening a window. Duplicate invocations in the same
  session must not restart pipelines that the user deliberately stopped.
- Report enabled status based on actual app-owned integration and target validity,
  rather than only a JSON boolean. Handle permission failures with rollback and
  a visible error. Preserve unrelated autostart entries and user-edited conflicts.
- Reconcile the existing optional `clearwave.service`: detect a legacy enabled
  unit and use one startup owner. Do not silently disable a user-managed service.
  Ensure intentional Quit remains exited, including under service supervision.
- Install/disable actions from the GUI are asynchronous. No root privileges or
  global startup configuration should be necessary.

### 3. Startup policy and persistence

- Add backward-compatible optional settings with all automatic capture off for
  existing configurations. Keep intended login policy separate from runtime state.
- Wait for the user's audio server and saved device with bounded retries. Never
  substitute a different microphone when the configured source is missing.
- Retry only startup readiness failures; after the user presses Stop/Quit, cancel
  pending retries. Prevent repeated starts on routine status polling/GUI reopen.
- Report unavailable devices through the tray and UI without blocking either.
  Apply temporary defaults only after the corresponding pipeline is ready.

### 4. Packaging and documentation

- Add original tray assets, dependency notices and any necessary build dependencies
  to Linux CI/AppImage. Verify running from the installed binary and AppImage path.
- Update desktop installer to include the assets/notices and support an existing
  startup entry when its executable changes. Do not enable startup just by installing.
- Update README with tray compatibility, tray drawer/pinning, login-vs-boot behavior,
  startup microphone consent, troubleshooting and disable/uninstall instructions.

## Verification and acceptance

- Unit tests: tray labels/menu availability from engine state; startup policy
  defaults and old configuration loading; executable/Exec escaping; idempotent
  enable/disable; partial writes/permissions; preservation of unrelated files.
- Private D-Bus test: fake SNI watcher receives one icon and DBusMenu; actions route
  correctly; watcher disappearance/reappearance does not stop audio or duplicate
  registrations; absence of session bus is nonfatal for headless use.
- Mock-audio lifecycle test: close GUI while processing -> tray remains; stop one
  path preserves the other; stop all cleans resources; Quit removes engine/socket/
  tray and does not respawn. Startup policy runs once, including duplicate launches,
  late server/device readiness, missing devices, and cancellation during retry.
- Verify autostart file in a temporary XDG directory first. Then test real desktop
  integration only within the user's authorized installation/testing scope. Preserve
  the user's current running engine/audio; use a distinct test instance when needed.
- Run formatting, strict Clippy, tests with/without desktop features, release build,
  desktop-file validation and AppImage smoke tests. Record actual results rather
  than assuming success from compilation.

### Required monitor/workspace constraint

All visible testing must use **the laptop monitor's second workspace**. Leave the
Acer monitor/workspaces untouched. Local config identifies the internal panel as
`eDP-1` and external as `HDMI-A-1`, but confirm live identity and workspace mapping
before launch; global workspace `2` is not automatically the laptop's second one.
Approved live reads subsequently confirmed laptop `eDP-1` uses workspace `2`
for the second slot; external Acer `HDMI-A-1` remained on workspace `11`.
Visible validation used only the laptop workspace; no monitor rules, bar settings
or persistent desktop configuration were changed.

Launch only test windows on that verified laptop workspace, capture only the app/
relevant laptop tray, restore any temporary focus/placement, and avoid restarting
the shell or editing persistent monitor rules. The host may mirror tray icons on
both displays; do not alter global bar settings just to hide a test icon on Acer.
Use generated audio and test devices before any separately requested microphone
listening test. If laptop workspace targeting is unavailable, run headless tests
and explicitly leave visual verification pending.

## Completion boundary

This document is the execution plan. Implementation is complete only when the tray
survives GUI close, Stop/Quit work through it, login startup is user-controllable,
selected processing starts without opening the GUI, and the relevant tests pass.
Do not mark those behaviors implemented based on this research alone.

## Implementation validation record (2026-11-18)

- Parent review: default suite 54 checks and headless suite 36 checks passed;
  strict locked Clippy passed. Mock audio includes independent controls, owned
  default restoration, startup once-only behavior, explicit defaults, missing
  device cancellation and late server readiness.
- Real Omarchy host: tray survived closing the test GUI; fake output/input started
  independently; microphone stop preserved output; StopAll removed fake modules;
  Quit exited without respawn. User audio was untouched.
- Live font/theme inspection confirmed system monospace and active theme; final
  compact/light-palette visual review is recorded by the parent reviewer.
- Private D-Bus regression uses a fake watcher and temporary XDG/mock audio;
  `scripts/test-private-tray.py` verifies real DBusMenu events and reconnects.
- `scripts/test-autostart-exec.py` exercises Desktop Entry Exec with GLib and a
  harmless temporary argument recorder, including moved-AppImage disable.
- Desktop file validation, shell syntax and whitespace checks passed. Final
  AppImage/release results are recorded in `docs/validation.md`.
