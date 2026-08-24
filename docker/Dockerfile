# Container image for the optional Leteo cloud service.
#
# The local product needs no container: it is a single binary writing to a
# SQLite file. This image exists for the shared cloud server, which is the only
# part that runs as a long-lived service against PostgreSQL.

FROM rust:1.97-bookworm AS build
WORKDIR /src

# rusqlite builds SQLite from source, so a C toolchain is required.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

# Cargo.lock is committed, so --locked keeps the image reproducible.
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# The migrations are `include_str!`d into the binary rather than read at run
# time, so they are a build input and not data. Left out, the build fails at
# `src/store/schema.rs` with "couldn't read
# src/store/../../migrations/0001_baseline_after_the_tables.sql" — which is what
# an image nobody builds looks like when somebody finally does. The fix has been
# in this file since the first commit here, so how long that lasted is not
# something this repository's history can show, and CI now builds this image on
# every pull request.
COPY migrations ./migrations
RUN cargo build --release --locked --bin leteo

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 leteo

COPY --from=build /src/target/release/leteo /usr/local/bin/leteo
COPY LICENSE NOTICE /usr/share/doc/leteo/

USER leteo
WORKDIR /home/leteo

# The cloud server binds every interface inside the container; publish it behind
# a TLS-terminating proxy, never directly.
ENV LETEO_CLOUD_HOST=0.0.0.0 \
    LETEO_CLOUD_PORT=8080
EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD ["leteo", "cloud", "health", "--server", "http://127.0.0.1:8080"]

ENTRYPOINT ["leteo"]
CMD ["cloud", "serve"]
