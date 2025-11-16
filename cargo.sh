#!/usr/bin/env bash

cargo fmt --all
cargo fix --allow-dirty --release --quiet
cargo clippy --fix --allow-dirty --quiet
cargo clippy --all-targets --all-features
