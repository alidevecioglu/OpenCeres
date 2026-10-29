# OpenCeres

A small, fast, open-source tool for **asteroid searching**: open a set of survey
images, blink them, find moving objects, measure them and write a
[Minor Planet Center](https://minorplanetcenter.net/) report.

OpenCeres is built for amateur search teams taking part in campaigns such as
those of the [International Astronomical Search Collaboration (IASC)](https://iasc.cosmosearch.org/),
where each team blinks sets of Pan-STARRS images to discover new asteroids.
It is named after Ceres, the first asteroid ever found (Piazzi, 1801).

> **Status: early development.** The image-processing core works and is tested
> on real Pan-STARRS images (FITS reading, star detection, sky coordinates, MPC
> report lines). The desktop app (native Rust, no web engine) opens image sets
> and zips, blinks them and makes box reports for teachers. Plate
> solving against Gaia and moving-object detection are in progress. Not yet
> ready for real campaign work.

## Goals

- **Tiny and fast.** A few megabytes, native on Apple Silicon, Windows and Linux,
  using every CPU core. The core library has no third-party dependencies.
- **Easy to use.** Drop an image set in and get candidates; confirm with the
  keyboard; one click to a ready-to-send report. No configuration files.
- **Accurate.** Positions must match established software on the same images
  before anyone relies on them.
- **Open.** Free to use, study and improve.

## Layout

```
crates/core   openceres-core: FITS, detection, WCS, time, MPC formatting (no dependencies)
crates/cli      openceres: command-line tool for checking the core on real data
crates/desktop  OpenCeres desktop app: egui (pure Rust, OpenGL), no web engine
```

## Building

Everything builds inside Docker; nothing needs to be installed on your machine
except Docker itself.

```sh
./dev check                      # core + CLI: format check, clippy, tests
./dev cargo test                 # tests only
./dev cargo build --release      # release build (Linux, in the container)
./dev shell                      # shell inside the dev container
```

Command-line tool:

```sh
./dev cargo run --release -p openceres-cli -- info  path/to/image.fits
./dev cargo run --release -p openceres-cli -- stars path/to/image.fits --limit 20
```

macOS app (built on the Mac itself, since Docker can't build macOS apps; needs
Apple's command-line tools). Rust is installed only inside
`.cache/macos-toolchain`; delete that folder to remove it.

```sh
./mac run                        # build OpenCeres.app (about 5 MB) and open it
./mac build                      # target/macos/release/OpenCeres.app
```

## Contributing

Issues and pull requests are welcome. Please run `./dev check` before opening a
pull request, and include tests for new behaviour — ideally against synthetic
images with known answers.

Do not commit real survey images or reports: survey data has its own usage
rules, and reports contain observer details. Keep them in `data/`, which is
git-ignored.

## Independence

OpenCeres is an independent project written from scratch. It is not affiliated
with, endorsed by, or derived from Astrometrica, its author, or IASC. Names of
other software and organisations are used only to describe compatibility.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in OpenCeres by you, as defined in the
Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
