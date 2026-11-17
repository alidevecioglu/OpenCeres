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
  windowed centroids with error propagation), `align` (star matching between
  frames for the blink), `wcs` (header TAN projection), `time` (JD ↔
  calendar; mid-exposure incl. Pan-STARRS SHMD*UTC), `mpc` (80-column lines,
  header lines, name/designation checks), `ades` (ADES 2022 PSV), `render`
  (display stretch).
- Report pipeline in core: `gaia` (VizieR I/355/gaiadr3 URLs + ASU-TSV
  parser; a mirror counts only if the reply holds Gaia rows; proper motion
  from J2016.0), `plate` (plate-constant polynomial in pixels → standard
  coordinates, order 1–3 chosen by 5-fold cross-validation, offset vote then
  triangle fallback for matching, `model_sigma` from (AᵀA)⁻¹), `photometry`
  (annulus sky, exact-area aperture, Gaia-G zero point; band `G`), `reduce`
  (one frame: detect → solve → zero point → measure marks). On IASC XY57
  frames: order 2, ~45 stars, 0.02–0.03″ RMS; header WCS is 1.3–7″ off.
- `crates/cli/` — `openceres info|stars|known|align|solve`; `solve` fetches
  Gaia with curl (or `--gaia file.tsv`) and prints fit, CV RMS per order,
  zero point and `--mark x,y` measurements.
- `crates/desktop/src/history.rs` — search history: one JSON file per search
  (id = file name, so imported ids are validated), merge-by-id import,
  CSV/JSON export. UI in `app/history_ui.rs`; `json.rs` is the shared
  hand-written parser/writer. Screenshots only touch the history when
  `OPENCERES_HISTORY_DIR` is set.
- `crates/desktop/src/app/objects_ui.rs` — detailed report: each moving
  object is marked frame by frame (snap to sources, predicted position;
  a plain click on the image starts one), one panel line per object; the
  search exports as JSON with or without PNG cutouts, and imported reports
  show their tracks on the open set. The side panel (`app.rs`
  `panel_sections`) has a fixed order — set, Objects, Report for IASC,
  Known objects (one collapsible line) — then Team / Imported reports /
  Box report only when they apply, and a footer (Saved, Notes, ⋯ menu);
  one accent button at a time, no text fields in the panel. Marks are stored
  in each frame's own pixels plus `frames` (file, DATE-OBS) so they map onto
  another computer's copy of the set.
- `crates/desktop/src/i18n.rs` — translations: `locales/<code>.txt`
  (`key = text`, `{name}` placeholders, embedded with `include_str!`),
  `tr("key")` / `tr_args("key", &[("name", v)])`, global language from
  `prefs.language`; falls back to English, then to the key. Tests check the
  files, placeholders and that `en.txt` has exactly the keys the code uses
  (literal `tr("…")` calls; `setting.<id>` keys translate registry labels).
  New UI text goes through `tr`; `TRANSLATING.md` is the translators' guide.
- `crates/desktop/src/app/onboarding_ui.rs` — the welcome window, shown
  until `prefs.onboarded`; reopened from Help → Welcome… and Settings →
  General. Reuses the Settings rows and the MPCORB download; captures show
  it only with `onboarding[=STEP]`.
- `crates/desktop/` — the desktop app: egui/eframe with the glow (OpenGL)
  backend, no web technology (user requirement: minimum resource use).
  Frames are R8 textures drawn by a small shader (invert is a uniform);
  reactive repaint only, idle CPU must stay ~0 %. Loads `PARALLEL` (2) frames
  at a time to bound peak memory. Measure RSS/footprint and binary size when
  changing dependencies or the loading path.
- MPC report in the app: `gaia.rs` (curl fetch, mirror order/fallback, cache
  in `<data>/Gaia/`, Settings mirror tests), `measure.rs` (background job:
  re-reads frames, runs `core::reduce`, keyed on marks + catalogue),
  `report.rs` (team details in `<data>/report.txt`, designations, checks,
  80-col + ADES text; `ALLOW_PREVIEW_COPY` = false: no copy without Gaia),
  UI in `app/mpc_ui.rs` (panel "Report for IASC", review window, Settings →
  Observer & report and the mirror rows). "Measured by" is the search's
  `searched_by`. Screenshots: `review`, `mpc=fetching|measuring|offline|
  failed|preview|missing|ready`.
- `crates/desktop/src/titlebar.rs`, `menubar.rs` — macOS only: the toolbar is
  the title bar (window buttons moved onto it, empty space drags), and a
  native menu bar whose items run the same actions as the keys. Only ⌘
  shortcuts go in the menu; plain-letter key equivalents would steal typing.

## Roadmap

1. Core on real IASC data: plate solve against Gaia DR3 (VizieR/TAP), compare
   positions with established software on the same images.
2. Known objects in the field (IMCCE SkyBoT), moving-object candidates across
   the image set.
3. App: blink viewer, candidate review, MPC report (80-column and ADES).
4. Group trial alongside existing software; confirm IASC accepts the reports.
