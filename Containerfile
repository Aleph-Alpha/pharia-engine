FROM rust:latest AS rust-builder
WORKDIR /build
# We use cargo chef to cache building our dependencies.
RUN cargo install cargo-chef
RUN cargo install cargo-auditable

# The planner produces a recipe for building our dependencies. It is important that it is separate
# from the builder, because dependencies are derived from our source, but most of the time the
# source changes, our dependencies do not. We want the build only to be dependent on the recipe.
# Not on the input (i.e. source) used to create it.
FROM rust-builder AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

# Builder provides the executable binary for the runner. We are careful to sort operations from "it
# changes less frequently" to "it changes more frequently".
FROM rust-builder AS builder
COPY --from=planner /build/recipe.json recipe.json

# Build dependencies only to cache them in a layer!
RUN cargo chef cook --release --recipe-path recipe.json

# Build application
COPY . .
RUN cargo auditable build --release

# Move rust binary in runtime container
FROM debian:12 AS runtime
RUN apt update && apt install openssl -y
COPY --from=builder /build/target/release/pharia-engine /usr/local/bin/pharia-engine
# use a random uid/gid to avoid running as root
USER 2000:2000
WORKDIR /app
CMD ["pharia-engine"]
