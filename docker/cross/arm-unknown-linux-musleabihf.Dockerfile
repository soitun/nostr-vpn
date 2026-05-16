# cross-rs `main` tag → latest tip (currently Ubuntu 20.04 / GLIBC 2.31).
# Pinned `0.2.5` (Ubuntu 16.04 / GLIBC 2.23) started failing 2026-05-16
# after `rust:stable` rolled forward and started emitting build-script
# binaries that link `GLIBC_2.28+`. cross-rs hasn't shipped a 0.2.6
# release yet, so we ride the `main` tag until they do.
FROM ghcr.io/cross-rs/armv7-unknown-linux-musleabihf:main AS headers
FROM ghcr.io/cross-rs/arm-unknown-linux-musleabihf:main

COPY --from=headers /usr/include/linux /usr/local/arm-linux-musleabihf/include/linux
