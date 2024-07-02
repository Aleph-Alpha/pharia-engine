FROM registry.gitlab.aleph-alpha.de/enterprise-readiness/shared-images/artifact-base/rust-builder as rust-builder
WORKDIR /build

# install plantuml dependencies
RUN apt update
RUN apt-get -y install default-jre graphviz

# install plantuml
RUN wget https://github.com/plantuml/plantuml/releases/download/v1.2024.5/plantuml-1.2024.5.jar -O /plantuml.jar
RUN printf -- '%s\n' '#!/bin/sh' > /usr/bin/plantuml
RUN printf -- '%s\n' 'exec /usr/bin/java -jar /plantuml.jar "$@"' >> /usr/bin/plantuml
RUN chmod +x /usr/bin/plantuml

# install mdbook-plantuml dependencies
RUN wget http://archive.ubuntu.com/ubuntu/pool/main/o/openssl/libssl1.1_1.1.0g-2ubuntu4_amd64.deb
RUN dpkg -i libssl1.1_1.1.0g-2ubuntu4_amd64.deb

# install cargo binstall
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash

# install mdbook and plugins
RUN cargo binstall -y mdbook
RUN cargo binstall -y mdbook-pagetoc
RUN cargo binstall -y mdbook-linkcheck
RUN cargo binstall -y mdbook-plantuml

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
# Build docs
RUN mdbook build doc

# Move rust binary in optimized runtime container
FROM registry.gitlab.aleph-alpha.de/enterprise-readiness/shared-images/artifact-base/rust-runtime
COPY --from=builder /build/target/release/pharia-kernel /usr/local/bin/pharia-kernel

# Move docs to runtime container
COPY --from=builder /build/doc/book/html /doc/book/html

ENV HOST=0.0.0.0
ENV PORT=8081

# use a random uid/gid to avoid running as root
USER 2000:2000
CMD ["pharia-kernel"]
