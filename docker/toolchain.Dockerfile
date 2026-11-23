# Toolchain for building and testing the Rust service in a container.
#
# The upstream `rust` image ships rustc and cargo but not clippy or rustfmt, so a
# `cargo clippy` run would fail. One image, one tag, referenced by scripts/cargo.sh.
FROM docker.io/library/rust:1-slim-trixie

RUN rustup component add clippy rustfmt

ENV CARGO_TERM_COLOR=always
