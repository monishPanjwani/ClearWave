# Supplemental dependency licenses

The license collector also copies notices embedded in Cargo registry packages.
Some published packages omit their repository-level license files. These copies
fill that gap; they are not FxSound-derived code or licenses:

- `egui/`: egui tag `0.31.1`, repository-root LICENSE-MIT and LICENSE-APACHE,
  https://github.com/emilk/egui/tree/0.31.1 . Applies to ecolor, eframe, egui,
  egui-winit, egui_glow, emath and epaint. Font notices are collected separately
  from the epaint_default_fonts crate, including its font subdirectories.
- `gl-rs/LICENSE`: repository-root Apache-2.0 text from
  https://github.com/brendanzab/gl-rs . gl_generator 0.14.0 and khronos_api 3.1.0
  declare Apache-2.0 and omit that text in their published archives.
- `profiling/`: repository commit `8271551172eb6fa4cba47369aedd93790c623df9`
  (the profiling 1.0.18 `.cargo_vcs_info.json` commit), root LICENSE-MIT and
  LICENSE-APACHE, https://github.com/aclysma/profiling .

Distribution bundles contain dependency metadata and these notices under
`dist/licenses/`; source code remains available at the declared repository URLs.
