#!/usr/bin/env bash
set -euo pipefail

export RUSTFLAGS="${RUSTFLAGS:-} --cfg loom"

cargo test --release -p poise-core --test loom_in_flight
cargo test --release -p poise-health --test loom_health
