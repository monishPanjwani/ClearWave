# Source provenance and distribution

ClearWave application code is MIT licensed (see LICENSE). Rust dependencies
retain their own licenses; distribute their notices with binaries. The microphone
model/implementation comes from `nnnoiseless` 0.5.2 (BSD-3-Clause), a Rust RNNoise
port, including its pretrained model. See its bundled `COPYING` and model notices
in the generated `dist/licenses/` bundle. No FxSound code, presets, images, fonts, drivers or assets are
included or translated into ClearWave.

## FxSound research reference (not bundled)

Feature mapping was researched against FxSound commit
[`d6d5b740f3efd29e5afcef6724bfd28f2827bc3b`](https://github.com/fxsound2/fxsound-app/tree/d6d5b740f3efd29e5afcef6724bfd28f2827bc3b).
Inspected files:

- `dsp/DfxDspEq.cpp`: API/registry synchronization, band frequency and gain controls.
- `dsp/include/DfxDsp.h`: effects and public DSP controls.
- `dsp/ptutil/DspUtil/GraphicEq/GraphicEqSet.cpp`: band count/frequency/gain,
  normalization, master gain, Q and volume-leveling setup.
- `GraphicEqProcess.cpp` and `GraphicEqInitSections.cpp` in that same directory:
  channel-dependent SOS processing and section initialization.
- `dsp/ptutil/Filt/FiltbiqdSos.cpp` and `FiltCalcBiqd.cpp`: actual upstream
  coefficient design (warped prototype/bandwidth and gain-dependent calculations).
- `dsp/ptutil/SOS/SosProcess.cpp`: actual filtering/volume leveling, including
  RMS/tonality/headroom analysis. This is substantially different from our
  fixed-threshold compressor.

These inspected files carry FxSound LLC copyright notices and AGPL-3.0-or-later
terms. They are research references only: ClearWave's original Rust filter bank,
control transitions, compressor and limiter are not translations or adaptations
of those implementations. Reading upstream does not license an AGPL port as MIT.
Any future port must preserve upstream notices, mark modifications, include the
AGPL license and provide corresponding source under compatible terms, including
when sharing builds with friends. No bit-identical or perceptual equivalence to
FxSound is claimed. ClearWave has five fixed EQ bands, original presets, broad
bass/clarity peaks and mid/side width; it does not implement FxSound Fidelity,
Ambience, Surround, DynamicBoost or its volume-leveling algorithm.

## Standard filter equations

The original implementation in `src/enhance.rs` uses the standard peaking EQ
and high-pass equations in Robert Bristow-Johnson's
[Audio EQ Cookbook (W3C Working Group Note, 8 June 2021)](https://www.w3.org/TR/2021/NOTE-audio-eq-cookbook-20210608/).
Peaking Q is fixed at 1; high-pass Q is 1/sqrt(2). Coefficients are normalized
by a0 and run in transposed direct form II with f64 filter states. The equations
were independently expressed in Rust; no document prose or example code was
copied. The W3C document acknowledges RBJ and carries the W3C Software and
Document license. Its copyright is © 2021 W3C (MIT, ERCIM, Keio, Beihang).

Gain uses 10^(dB/20). Compression uses peak excess above threshold multiplied by
(1 - 1/ratio), with linked stereo gain and attack/release smoothing. Limiting is
an instantaneous sample-peak attenuation with slow release and a 0.98 ceiling;
it is not a true-peak or lookahead mastering limiter. Mid/side width multiplies
(L-R)/2 before recombination. These are standard equations, with independently
chosen implementation and tuning. No universal best EQ or hearing-protection
claim is made.

## Bundled Omarchy color palettes

The eight TOML palettes in `assets/palettes/` are snapshots of
`themes/<palette-id>/colors.toml` from [basecamp/omarchy](https://github.com/basecamp/omarchy/tree/f4378f0de5b44d331ee943746a97872b718a6c18),
revision `f4378f0de5b44d331ee943746a97872b718a6c18`, retrieved 2026-11-17.
IDs: tokyo-night, catppuccin, catppuccin-latte, nord, rose-pine, gruvbox,
everforest, flexoki-light. The Omarchy repository's MIT permission/copyright
notice is preserved in `licenses/omarchy/LICENSE` and bundled by the build script.
No theme scripts, wallpapers, fonts or logos are copied. The palette data is
compiled into the application for offline portability. Text/secondary/semantic
colors may be derived for contrast; card surfaces use lighter_background and
missing fields fall back to documented defaults in `src/theme.rs`.

## Background indicator

The original app art in `assets/clearwave.svg`/`.png` and wave/status glyphs in
`assets/tray/` and `src/tray.rs` are ClearWave
MIT assets. SNI/DBusMenu uses ksni 0.3.3 (Unlicense), with its transitive
dependencies' notices collected in `dist/licenses/` by `scripts/collect-licenses.py`.
No desktop panel artwork or shell integration code is copied.
