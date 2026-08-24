# Build stage
FROM rust:1.98 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

# Runtime stage: distroless-style static binary
FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/tg-bridge /usr/local/bin/tg-bridge
USER nonroot:nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/tg-bridge"]
