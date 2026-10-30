# CLAUDE.md

Guidance for AI coding assistants working in this repository.

## Project

OpenCeres is an open-source asteroid-search tool (MIT OR Apache-2.0) for amateur
teams in campaigns like IASC's: blink 3–4 Pan-STARRS FITS images, find moving
objects, measure positions, write MPC reports. Product priorities, in order:
**accuracy, small size, speed, ease of use.**

## Hard rules

- **Clean-room.** Never decompile, reverse-engineer, translate or copy code, UI
  text, file formats, icons or help from Astrometrica or any other closed
  program. Implement from published science and public specifications (FITS
  standard, FITS WCS papers, MPC format docs, Meeus, SExtractor papers).
- **Docker only.** Do not install toolchains on the host. Run everything via
  `./dev ...` (see `compose.yaml`). The one exception is the macOS app, which
  `./mac` builds with a Rust toolchain kept inside `.cache/macos-toolchain`
  (never in `~/.cargo` or the shell profile).
- **No real data in git.** Survey images, reports and observer details live in
  `data/` (git-ignored). Tests use synthetic images with known answers.
- **`openceres-core` stays dependency-free.** Adding a crate needs a strong
  reason (e.g. something we cannot reasonably write and test ourselves) and
  a note in the PR. Size matters: check release binary size when it changes.
- Do not commit, push or open PRs unless asked.

## Commands

```sh
./dev check                 # all crates: fmt --check, clippy -D warnings, tests (run before finishing)
./mac run                   # desktop app: build and open "OpenCeres Dev.app" on the Mac
./dev cargo test -p openceres-core
./dev cargo run --release -p openceres-cli -- stars data/<file>.fits
```

## Conventions

- Pixel coordinates are **0-based**, (0, 0) = centre of the first stored pixel.
  FITS is 1-based; convert only inside `wcs.rs`. FITS stores the bottom row
  first; `render::grayscale` flips rows for display.
- Angles: degrees in public APIs, radians only inside computations. RA in [0, 360).
- Times: UTC Julian dates (`f64`) internally; `time::CalendarDate` for MPC output.
- MPC 80-column lines must be exactly 80 characters; round in integer units of
  the last printed digit so values never print as 60 seconds.
- Accuracy claims need evidence: prefer statistical tests over many noise
  realisations (see `detect::tests::centroid_precision`) to single examples, and
  never loosen a tolerance just to make a test pass — find out why first.
- Match the surrounding style: small modules, doc comments that explain *why*,
  few inline comments, `rustfmt` defaults.

## Layout

- `crates/core/src/` — `fits` (reader), `detect` (background, sources,
  windowed centroids), `wcs` (TAN projection), `time` (JD ↔ calendar),
  `mpc` (80-column format), `render` (display stretch).
- `crates/cli/` — `openceres info|stars` for checking the core on real files.
- `crates/desktop/` — the desktop app: egui/eframe with the glow (OpenGL)
  backend, no web technology (user requirement: minimum resource use).
  Frames are R8 textures drawn by a small shader (invert is a uniform);
  reactive repaint only, idle CPU must stay ~0 %. Loads `PARALLEL` (2) frames
  at a time to bound peak memory. Measure RSS/footprint and binary size when
  changing dependencies or the loading path.

## Roadmap

1. Core on real IASC data: plate solve against Gaia DR3 (VizieR/TAP), compare
   positions with established software on the same images.
2. Known objects in the field (IMCCE SkyBoT), moving-object candidates across
   the image set.
3. App: blink viewer, candidate review, MPC report (80-column and ADES).
4. Group trial alongside existing software; confirm IASC accepts the reports.
