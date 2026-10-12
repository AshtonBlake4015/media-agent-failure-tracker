#!/usr/bin/env sh
set -eu

: "${INFRAI_API_KEY:?set INFRAI_API_KEY before running this demo}"
cargo run --bin media_agent -- --simulate-processing-failure

