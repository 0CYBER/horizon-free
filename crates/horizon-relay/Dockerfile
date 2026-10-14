# The Play with friends relay, for anyone to run (docs/play-with-friends.md, "Running your own relay").
# Build context: the repository root (the workspace's manifests and lockfile):
#   docker build -f crates/horizon-relay/Dockerfile -t horizon-relay .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p horizon-relay \
    && cp target/release/horizon-relay /horizon-relay

FROM debian:bookworm-slim
COPY --from=build /horizon-relay /usr/local/bin/horizon-relay
ENV PORT=7000
EXPOSE 7000
USER nobody
CMD ["horizon-relay"]
