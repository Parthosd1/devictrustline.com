# API-only image (no web app), used by the local docker-compose.yml.
FROM rust:1-slim-bookworm AS build
WORKDIR /api
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --release --locked && rm -rf src
COPY migrations ./migrations
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /api/target/release/devicetrustline /usr/local/bin/devicetrustline
ENV APP_ENV=production PORT=4000
EXPOSE 4000
HEALTHCHECK --interval=30s --timeout=5s CMD ["devicetrustline", "healthcheck"]
ENTRYPOINT ["devicetrustline"]
CMD ["serve"]
