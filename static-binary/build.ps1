rm -Recurse src
mkdir src
cp -Recurse ..\src src/src
cp ..\Cargo.toml src/
cp ..\Cargo.lock src/
cp ..\rust-target-version .

docker build --tag build-aelita-hosted -f build.dockerfile .
docker rm build-aelita-hosted
docker create --name build-aelita-hosted build-aelita-hosted
docker cp build-aelita-hosted:/aelita/target/x86_64-unknown-linux-musl/release/aelita .
docker cp build-aelita-hosted:/usr/local/musl/etc/ssl/cert.pem .
