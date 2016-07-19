#!/bin/sh
set -e

# Copy source code to Docker working area.
rm -rf src
mkdir src
cp -rf ../src src/src
cp -rf ../Cargo.toml src/
cp -rf ../Cargo.lock src/
cp -rf ../rust-target-version .

docker build --tag=build-aelita-hosted -f build.dockerfile .
docker rm build-aelita-hosted || true
docker create --name=build-aelita-hosted build-aelita-hosted
docker cp \
  build-aelita-hosted:/aelita/target/x86_64-unknown-linux-musl/release/aelita .
docker cp build-aelita-hosted:/usr/local/musl/etc/ssl/cert.pem .

