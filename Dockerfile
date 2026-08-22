# Important: This file is provided for demonstration purposes and may NOT be suitable for production use.
# The maintainers of electrs are not deeply familiar with Docker, so you should DYOR.
# If you are not familiar with Docker either it's probably be safer to NOT use it.

FROM debian:bookworm-slim AS base
RUN apt-get update -qqy
RUN apt-get install -qqy librocksdb-dev curl

### Electrum Rust Server ###
FROM base AS electrs-build
RUN apt-get install -qqy cargo clang cmake

# Install electrs
WORKDIR /build/electrs
COPY . .
ENV ROCKSDB_INCLUDE_DIR=/usr/include
ENV ROCKSDB_LIB_DIR=/usr/lib
RUN cargo install --locked --path .

FROM base AS result
# Copy the binaries
COPY --from=electrs-build /root/.cargo/bin/electrs /usr/bin/electrs

WORKDIR /

RUN groupadd --gid 568 electrs \
    && useradd --uid 568 --gid electrs --home-dir /home/electrs --create-home \
        --shell /usr/sbin/nologin electrs \
    && install --directory --owner=electrs --group=electrs /data

ENV HOME=/home/electrs \
    ELECTRS_DB_DIR=/data

EXPOSE 50001 4225

USER electrs
ENTRYPOINT ["/usr/bin/electrs"]
