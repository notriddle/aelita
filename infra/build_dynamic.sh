#!/bin/sh
set -ex
. infra/install_rust.sh
cargo build
cargo test

