# The Fly.io server: just the API (`--features server`). No web UI is served.
FROM rust:1-bookworm AS build
WORKDIR /app
COPY . .
RUN cargo build --release --features server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/deadline /usr/local/bin/deadline
ENV IP=0.0.0.0 \
    PORT=8080 \
    DEADLINE_DB=/data/deadline.db
EXPOSE 8080
CMD ["deadline"]
