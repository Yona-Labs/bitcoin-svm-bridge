FROM ubuntu:22.04 AS builder

WORKDIR /build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq \
    && apt-get install -qq -y \
       build-essential \
       git \
       curl \
       wget \
       jq \
       pkg-config \
       python3-pip \
       libssl-dev \
       libudev-dev \
       gcc-multilib

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


COPY . .

ENV PATH=$PATH:$NVM_DIR/versions/node/$NODE_VERSION/bin
RUN yarn install

ENV PATH=$PATH:/root/.local/share/solana/install/active_release/bin
#RUN anchor build

RUN cd block_relayer \
    && cargo build --release

#RUN find / -name block_relayer -type f


FROM rust:1.91-slim-trixie AS app_block_relayer

WORKDIR /app

ENV PATH=$PATH:/app/bin

COPY --from=builder /build/block_relayer/target/release/block_relayer ./bin/block_relayer
#COPY --from=builder /workdir/programs/btc-relay /app/bin/btc_relay

ENTRYPOINT [ "./bin/block_relayer" ]
