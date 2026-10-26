# Production image: builds the web app and the Rust API; the API also serves the app.
FROM node:22-alpine AS web
WORKDIR /web
COPY package.json package-lock.json ./
RUN npm ci
COPY index.html vite.config.js style.css ./
COPY src ./src
RUN npm run build

FROM rust:1-slim-bookworm AS api
WORKDIR /api
# Build dependencies first so code changes do not rebuild them.
COPY api/Cargo.toml api/Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked && rm -rf src
COPY api/migrations ./migrations
COPY api/src ./src
RUN touch src/main.rs && cargo build --release --locked

# The Windows device companion, offered for download from the Devices page.
FROM rust:1-slim-bookworm AS agent
RUN apt-get update && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-pc-windows-gnu
WORKDIR /agent
COPY agent/Cargo.toml agent/Cargo.lock ./
COPY agent/.cargo ./.cargo
COPY agent/src ./src
RUN cargo build --release --locked --target x86_64-pc-windows-gnu
# An empty directory owned by the runtime user, so a fresh volume mounted here is writable.
RUN mkdir -p /data/attachments

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
ENV APP_ENV=production STATIC_DIR=/app/web PORT=4000 ATTACHMENTS_DIR=/data/attachments
COPY --from=api /api/target/release/devicetrustline /usr/local/bin/devicetrustline
COPY --from=web /web/dist ./web
COPY --from=agent /agent/target/x86_64-pc-windows-gnu/release/devicetrustline-agent.exe ./web/downloads/devicetrustline-agent.exe
COPY --from=agent --chown=65532:65532 /data /data
VOLUME /data
EXPOSE 4000
HEALTHCHECK --interval=30s --timeout=5s CMD ["devicetrustline", "healthcheck"]
ENTRYPOINT ["devicetrustline"]
CMD ["serve"]
