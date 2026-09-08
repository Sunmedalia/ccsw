FROM rust:1.95.0-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends python3 && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy
WORKDIR /work
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests
RUN cargo fmt --check && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --all-targets && cargo build --locked
RUN cp target/debug/ccsw /usr/local/bin/ccsw-test && useradd -m -u 10001 testuser
CMD ["python3", "/work/tests/docker/uninstall_safety.py"]
