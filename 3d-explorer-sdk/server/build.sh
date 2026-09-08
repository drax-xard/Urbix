#!/bin/sh
# server/build.sh
#
# Builds the micro-server against the shipped SDK (static lib).
# Run from 3d-explorer-sdk/:
#   ./server/build.sh
set -e
cd "$(dirname "$0")/.."

cc -O2 -Wall -Wextra -I sdk/include server/serve.c sdk/lib/liburbix.a \
   -framework Security -framework CoreFoundation -lm \
   -o server/serve

echo "built server/serve"
echo "run: ./server/serve --port 8311 --seed 445566 --web server/www"