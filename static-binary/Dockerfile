FROM scratch
ENV RUST_LOG aelita=debug
ENV RUST_BACKTRACE 1
ADD cert.pem /etc/ssl/cert.pem
ADD aelita /aelita
ENTRYPOINT [ "/aelita", "-12" ]

