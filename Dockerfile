# OpenCeres development image: everything needed to build and test,
# so nothing has to be installed on the host.
#
# macOS .app bundles cannot be built in Linux containers (Apple toolchain);
# those are built by CI on macOS runners.
FROM rust:1.98.1-slim-bookworm

RUN rustup component add rustfmt clippy

WORKDIR /work
