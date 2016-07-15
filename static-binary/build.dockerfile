FROM buildpack-deps:stretch
ENV PATH /usr/local/musl/bin/:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
WORKDIR /tmp
# Install musl (everything needs it)
ADD musl-target-version /musl-target-version
RUN curl `cat /musl-target-version` | tar -xzf - && \
    cd musl-* && \
    ./configure CFLAGS="-fPIE" && \
    make -j2 && \
    make install
# Install kernel headers (libressl needs them)
ADD kheaders-target-version /kheaders-target-version
RUN curl `cat /kheaders-target-version` | tar -xJf - && \
    cd kernel-headers-* && \
    make install ARCH=x86_64 DESTDIR=/usr/local/musl/ prefix=
# Install libressl (hyper needs it)
ADD libressl-target-version /libressl-target-version
RUN curl `cat /libressl-target-version` | tar -xzf - && \
    cd libressl-* && \
    ./configure --disable-dynamic --prefix= --host=x86_64-unknown-linux-musl CC=musl-gcc && \
    make -j2 && \
    make install DESTDIR=/usr/local/musl/
# Install sqlite (Aelita uses it)
ADD sqlite-target-version /sqlite-target-version
RUN curl `cat /sqlite-target-version` | tar -xzf - && \
    cd sqlite-* && \
    ./configure --disable-dynamic --prefix= --host=x86_64-unknown-linux-musl CC=musl-gcc && \
    make -j2 && \
    make install DESTDIR=/usr/local/musl/
# Install Rust (Aelita and the bulk of its deps need it)
ADD rust-target-version /rust-target-version
RUN curl `cat /rust-target-version` | tar -xzf - && \
    cd rust-nightly-* && \
    ./install.sh && \
    std=`sed s:rust-nightly-x86_64-unknown-linux-gnu:rust-std-nightly-x86_64-unknown-linux-musl: /rust-target-version` && \
    curl $std | tar -xzf - && \
    cd rust-std-* && \
    ./install.sh
# The nasty rustc hack to keep our libraries coming from static.
ADD rustc /my-bin/rustc
ENV PATH /my-bin:/usr/local/musl/bin/:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin
RUN chmod +x /my-bin/rustc
# Build Aelita
ADD src/ /aelita
WORKDIR /aelita
ENV CC musl-gcc
ENV PKG_CONFIG_PATH /usr/local/musl/lib/pkgconfig/
ENV PKG_CONFIG_LIBDIR /usr/local/musl/lib/pkgconfig/
ENV PKG_CONFIG_ALL_STATIC 1
RUN rm -rf /usr/lib/pkgconfig && \
    rm -f /usr/local/musl/lib/*.so.* /usr/local/musl/lib/*.so && \
    cargo build -v --target x86_64-unknown-linux-musl --release
RUN strip -s target/x86_64-unknown-linux-musl/release/aelita

