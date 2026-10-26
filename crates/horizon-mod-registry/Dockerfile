# Built from the repository root: docker build -f crates/horizon-mod-registry/Dockerfile .
FROM rust:1-slim AS build
WORKDIR /src
# The root manifest only for the shared crate's workspace settings (edition, lints).
COPY Cargo.toml Cargo.toml
COPY crates/horizon-mods crates/horizon-mods
COPY crates/horizon-mod-registry crates/horizon-mod-registry
RUN cargo build --release --locked --manifest-path crates/horizon-mod-registry/Cargo.toml

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/crates/horizon-mod-registry/target/release/horizon-mod-registry /usr/local/bin/
USER nobody
CMD ["horizon-mod-registry"]
