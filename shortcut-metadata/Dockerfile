FROM rust:1.93-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim
COPY --from=build /src/target/release/solid-workbench-metadata /usr/local/bin/
USER 65534:65534
ENV METADATA_BIND=0.0.0.0:4175
EXPOSE 4175
ENTRYPOINT ["/usr/local/bin/solid-workbench-metadata"]
