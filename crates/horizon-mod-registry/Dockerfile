# Built from the repository root: docker build -f crates/horizon-mod-registry/Dockerfile .
FROM rust:1-slim AS build
WORKDIR /src
# The root manifest only for the shared crates' workspace settings (edition, lints).
COPY Cargo.toml Cargo.toml
# Every crate's sources, though only the registry's own workspace compiles: the crates its Cargo.lock builds from a
# path, and what horizon-mod-vocab's build script reads to generate the mod API (the game's hook sources under
# crates/horizon-client/src/game and the crate each hook target names). tests/test_service_images.py checks this.
# .dockerignore leaves out build folders.
COPY crates crates
# The files outside crates/ that the build reads: the generator's data and what a crate includes (tests/test_service_images.py
# derives the list from their sources).
COPY tools/layering/mod_vocab_state.json tools/layering/mod_vocab_state.json
COPY tools/mixins/narrow-reference-rule.patch tools/mixins/narrow-reference-rule.patch
RUN cargo build --release --locked --manifest-path crates/horizon-mod-registry/Cargo.toml

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/crates/horizon-mod-registry/target/release/horizon-mod-registry /usr/local/bin/
ENV REGISTRY_DB=/data/registry.sqlite3
# Railway mounts the ratings' volume at /data owned by root: hand it to nobody, then run as nobody (setpriv is
# util-linux's, in every Debian image).
CMD ["sh", "-c", "mkdir -p /data && chown nobody:nogroup /data && exec setpriv --reuid=nobody --regid=nogroup --clear-groups horizon-mod-registry"]
