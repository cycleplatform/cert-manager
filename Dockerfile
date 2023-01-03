FROM docker.io/library/rust:1.66-alpine as builder
RUN apk add --no-cache musl-dev  openssl-dev openssl-libs-static pkgconf git

# Set `SYSROOT` to a dummy path (default is /usr) because pkg-config-rs *always*
# links those located in that path dynamically but we want static linking, c.f.
# https://github.com/rust-lang/pkg-config-rs/blob/54325785816695df031cef3b26b6a9a203bbc01b/src/lib.rs#L613
ENV SYSROOT=/dummy

WORKDIR /wd
COPY . /wd
RUN cargo build --bins --release

FROM scratch
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /wd/target/release/cycle-certs /
CMD ["./cycle-certs"]