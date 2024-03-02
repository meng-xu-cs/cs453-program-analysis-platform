#!/bin/bash -e

# paths
SCRIPT_DIR=$(dirname "$0")
BASE_DIR=$(dirname "$SCRIPT_DIR")

# install packages
sudo apt-get update
sudo apt-get upgrade -y
sudo apt-get install -y \
  build-essential \
  wget curl \
  virtualbox vagrant
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# install docker (if not installed yet)
if [ $(getent group docker) ]; then
  echo "skip docker installation"
else
  curl -fsSL https://get.docker.com -o get-docker.sh
  sudo sh get-docker.sh
  rm get-docker.sh

  sudo usermod -aG docker "$USER"
  newgrp docker
  docker run hello-world
fi

# build all necessary docker images
cd "$BASE_DIR/worker"
cargo run
cd -
