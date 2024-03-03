#!/bin/bash -e

# settings
export DEBIAN_FRONTEND=noninteractive

# paths
SCRIPT_FILE=$(realpath "$0")
SCRIPT_DIR=$(dirname "$SCRIPT_FILE")
BASE_DIR=$(dirname "$SCRIPT_DIR")

# install packages
sudo apt-get update
sudo apt-get upgrade -y
sudo apt-get install -y \
  build-essential wget curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# install docker (if not installed yet)
if [ $(getent group docker) ]; then
  echo "skip docker installation"
else
  curl -fsSL https://get.docker.com -o get-docker.sh
  sudo sh get-docker.sh
  rm get-docker.sh
  sudo usermod -aG docker "$USER"
fi

# tweak settings
sudo apt-get purge -y apport
sudo sysctl -w kernel.core_pattern=core.%e.%p

# refresh shell
source ~/.profile

# build all necessary docker images
cd "$BASE_DIR/worker"
newgrp docker <<BUILD
cargo run
BUILD
cd -

# all set!
echo "==== END OF PROVISION ==="
