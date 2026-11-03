# OpenCeres development image: everything needed to build and test,
# so nothing has to be installed on the host.
#
# macOS .app bundles cannot be built in Linux containers (Apple toolchain);
# those are built by CI on macOS runners.
FROM rust:1.98.1-slim-bookworm

RUN rustup component add rustfmt clippy

# The app goes online through the system curl (as on macOS and Windows);
# the team-server tests send real requests to a local test server with it.
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /work
