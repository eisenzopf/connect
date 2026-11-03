# Native ARM64 component testing on the owner's Mac Studio. No host mounts.
FROM ubuntu:24.04@sha256:18b33ff3829707f31ada654a963b9ee35f2e6f0241b964bd17a22a9b330cb63d
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates curl xz-utils git python3 build-essential clang cmake pkg-config \
    libopus-dev libssl-dev ffmpeg procps iproute2 tcpdump sudo \
    && dpkg-query -W > /usr/local/share/connect-build-packages.tsv \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
ENV PATH=/opt/cargo/bin:/opt/node/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    -y --default-toolchain 1.97.1 --profile minimal --component rustfmt
RUN mkdir /tmp/node-download && cd /tmp/node-download \
    && curl --proto '=https' --tlsv1.2 -fsSLO https://nodejs.org/dist/v22.16.0/node-v22.16.0-linux-arm64.tar.xz \
    && curl --proto '=https' --tlsv1.2 -fsSLO https://nodejs.org/dist/v22.16.0/SHASUMS256.txt \
    && grep ' node-v22.16.0-linux-arm64.tar.xz$' SHASUMS256.txt | sha256sum -c - \
    && mkdir /opt/node && tar -xJf node-v22.16.0-linux-arm64.tar.xz -C /opt/node --strip-components=1 \
    && rm -rf /tmp/node-download
RUN rustup component add clippy
RUN useradd --create-home --uid 1001 --shell /bin/bash connect \
    && mkdir /workspace && chown connect:connect /workspace /opt/cargo /opt/rustup \
    && printf 'connect ALL=(root) NOPASSWD: /usr/bin/tcpdump\n' > /etc/sudoers.d/connect-capture \
    && chmod 0440 /etc/sudoers.d/connect-capture
USER connect
WORKDIR /workspace
ENV CARGO_BUILD_JOBS=4 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_INCREMENTAL=0
CMD ["sleep", "infinity"]
