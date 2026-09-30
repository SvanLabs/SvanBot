# SvanBot in a container: the fleet, learner and analyst, and the built dashboard (#16).
#   docker run -d -p 5000:5000 -v "$PWD/.env:/svanbot/.env:ro" -v svanbot-data:/svanbot/artifacts \
#     ghcr.io/svanlabs/svanbot:latest
# Use the compiler's baseline CPU for the selected platform, rather than the build host's ISA.

FROM node:24-bookworm AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
COPY docs/ /src/docs/
RUN npm run build

FROM rust:1.98.1-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
ENV RUSTFLAGS=""
RUN . scripts/resources.sh && cargo build --release --workspace --bins

FROM debian:bookworm-slim
# These are the tools the scripts copied below actually run, and they have to be installed *here*:
# the build stage's packages do not reach the image. A missing one fails quietly, because the
# scripts redirect their own output to a log — the results monitor could not start at all without
# python3, and status.sh needs sqlite3 and curl for the fleet summary (#582).
RUN apt-get update \
 && apt-get install -y --no-install-recommends bash ca-certificates curl procps python3 sqlite3 zstd \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /svanbot
COPY scripts/ scripts/
COPY docs/ docs/
COPY .env.example Cargo.toml ./
COPY --from=build /src/target/release/ target/release/
COPY --from=web /src/web/dist/ web/dist/
RUN find target/release -maxdepth 1 -type f ! -perm -u+x -delete \
 && rm -rf target/release/build target/release/deps target/release/incremental target/release/.fingerprint
# Reachable from outside the container; changes still need SVANBOT_WEB__OPERATOR_TOKEN off loopback.
ENV SVANBOT_WEB__HOST=0.0.0.0
EXPOSE 5000
VOLUME ["/svanbot/artifacts"]
CMD ["scripts/container-start.sh"]
