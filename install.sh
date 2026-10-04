#!/bin/sh
# Builds mono-greeter as you, then installs it with sudo (see dist/install-prebuilt.sh).
#   ./install.sh                 into /usr/local
#   PREFIX=/usr ./install.sh     into /usr
set -eu
cd "$(dirname "$0")"

cargo build --release --locked
exec sudo env PREFIX="${PREFIX:-/usr/local}" sh dist/install-prebuilt.sh --from target/release
