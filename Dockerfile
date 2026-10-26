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

FROM gcr.io/distroless/cc-debian12:nonroot
WORKDIR /app
ENV APP_ENV=production STATIC_DIR=/app/web PORT=4000
COPY --from=api /api/target/release/devicetrustline /usr/local/bin/devicetrustline
COPY --from=web /web/dist ./web
EXPOSE 4000
HEALTHCHECK --interval=30s --timeout=5s CMD ["devicetrustline", "healthcheck"]
ENTRYPOINT ["devicetrustline"]
CMD ["serve"]
