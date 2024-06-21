FROM registry.gitlab.aleph-alpha.de/enterprise-readiness/shared-images/artifact-base/rust-builder as rust-builder
WORKDIR /build

FROM rust-builder AS planner
COPY . .
RUN cargo chef prepare  --recipe-path recipe.json

FROM rust-builder AS builder
COPY --from=planner /build/recipe.json recipe.json

# Build dependencies only to cache them in a layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo auditable build --release
RUN rustup target add wasm32-wasi
RUN ./build-skill.sh

# Move rust binary in optimized runtime container
FROM registry.gitlab.aleph-alpha.de/enterprise-readiness/shared-images/artifact-base/rust-runtime
COPY --from=builder /build/target/release/pharia-kernel /usr/local/bin/pharia-kernel
COPY --from=builder /build/skills /skills

ENV HOST=0.0.0.0
ENV PORT=8081

# use a random uid/gid to avoid running as root
USER 2000:2000
CMD ["pharia-kernel"]
