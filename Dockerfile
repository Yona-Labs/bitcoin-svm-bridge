FROM ubuntu:22.04 AS base

WORKDIR /build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq \
    && apt-get install -qq -y --no-install-recommends \
       build-essential \
       git \
       curl \
       wget \
       jq \
       pkg-config \
       python3-pip \
       libssl-dev \
       libudev-dev \
       gcc-multilib \
    && rm -rf /var/lib/apt/lists/* 

ENV PATH=$PATH:/root/.cargo/bin
RUN curl https://sh.rustup.rs -sfo rustup.sh \
    && sh rustup.sh -y \
    && rustup component add rustfmt clippy \
    && rustup default stable

ENV NODE_VERSION=v24.11.0 \
    NVM_CLI=v0.40.3 \
    NVM_DIR=/root/.nvm
RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/$NVM_CLI/install.sh | bash \
    && . $NVM_DIR/nvm.sh \
    && nvm install ${NODE_VERSION} \
    && nvm use ${NODE_VERSION} \
    && nvm alias default node \
    && npm install -g yarn

ENV SOLANA_CLI=v2.3.11
RUN sh -c "$(curl -sSfL https://release.anza.xyz/${SOLANA_CLI}/install)"

ENV ANCHOR_CLI=v0.31.1
RUN cargo install --git https://github.com/coral-xyz/anchor --tag ${ANCHOR_CLI} anchor-cli --locked



FROM base AS builder

WORKDIR /build

COPY . .

#ENV PATH=$PATH:$NVM_DIR/versions/node/$NODE_VERSION/bin
#RUN yarn install

ENV PATH=$PATH:/root/.local/share/solana/install/active_release/bin
#RUN anchor build

RUN cd block_relayer \
    && cargo build --release

#RUN find / -name block_relayer -type f



FROM rust:1.91-slim-trixie AS app_block_relayer

WORKDIR /app

COPY --from=builder /build/block_relayer/target/release/block_relayer /app/block_relayer
#COPY --from=builder /workdir/programs/btc-relay /app/bin/btc_relay

RUN useradd -d /app -s /bin/bash -c "Yona user" yona \
    && chown yona -R /app \
    && chmod 0775 /app \
    && chmod +x /app/block_relayer

ENV RUST_LOG=info \
    RUST_BACKTRACE=1

#USER yona

ENTRYPOINT [ "./block_relayer" ]
