#!/bin/sh
set -ex
. infra/install_rust.sh
cd static-binary
sh build.sh
cd ..
mkdir -p target/debug
cp static-binary/aelita target/debug
cargo test round_trip

