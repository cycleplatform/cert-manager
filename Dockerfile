FROM docker.io/library/rust:1-alpine AS builder
RUN apk add --no-cache musl-dev pkgconf git
ENV SYSROOT=/dummy
WORKDIR /cycle
COPY . .
RUN cargo build --bins --release

FROM scratch
VOLUME ["/certs"]
COPY --from=builder /cycle/target/release/cycle-certs /usr/local/bin/cycle-certs
ENTRYPOINT ["/usr/local/bin/cycle-certs", "--path=/certs", "--config=/certs/config"]