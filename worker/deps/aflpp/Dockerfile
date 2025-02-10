FROM ubuntu:22.04

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get -y install --no-install-recommends \
    curl git build-essential automake cmake python3-dev

RUN git clone https://github.com/AFLplusplus/AFLplusplus.git && \
    cd AFLplusplus && git checkout v4.31c &&  \
    make source-only && sudo make install
