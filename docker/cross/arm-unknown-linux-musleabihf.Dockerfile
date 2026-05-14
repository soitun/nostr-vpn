FROM ghcr.io/cross-rs/armv7-unknown-linux-musleabihf:0.2.5 AS headers
FROM ghcr.io/cross-rs/arm-unknown-linux-musleabihf:0.2.5

COPY --from=headers /usr/include/linux /usr/local/arm-linux-musleabihf/include/linux
