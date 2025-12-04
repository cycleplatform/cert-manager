FROM docker.io/library/rust:1.91-alpine AS builder
RUN apk add --no-cache musl-dev pkgconf git

# Force pkg-config-rs to avoid linking from /usr
ENV SYSROOT=/dummy

WORKDIR /cycle
COPY . .
RUN cargo build --bins --release


FROM scratch AS minimal
VOLUME ["/certs"]
COPY --from=builder /cycle/target/release/cycle-certs /
ENTRYPOINT ["/cycle-certs", "--path=/certs", "--config=/certs/config"]

FROM alpine
RUN apk add --no-cache curl

VOLUME ["/certs"]

COPY --from=builder /cycle/target/release/cycle-certs /usr/local/bin/cycle-certs

ENTRYPOINT ["/usr/local/bin/cycle-certs", "--path=/certs", "--config=/certs/config"]

