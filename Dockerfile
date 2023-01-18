FROM docker.io/library/rust:1.66-alpine as builder
RUN apk add --no-cache musl-dev pkgconf git

# Set `SYSROOT` to a dummy path (default is /usr) because pkg-config-rs *always*
# links those located in that path dynamically but we want static linking, c.f.
# https://github.com/rust-lang/pkg-config-rs/blob/54325785816695df031cef3b26b6a9a203bbc01b/src/lib.rs#L613
ENV SYSROOT=/dummy

WORKDIR /cycle
COPY . .
RUN cargo build --bins --release

# Image for docker-in-docker builds where the host may want to restart another
# container etc.
FROM alpine:3.10  as dnd
RUN apk add --update docker openrc
RUN rc-update add docker boot
VOLUME ["/certs"]
COPY --from=builder /cycle/target/release/cycle-certs /
ENTRYPOINT ["./cycle-certs", "--path=/certs", "--config=/certs/config"]

FROM scratch
VOLUME ["/certs"]
COPY --from=builder /cycle/target/release/cycle-certs /
ENTRYPOINT ["./cycle-certs", "--path=/certs", "--config=/certs/config"]