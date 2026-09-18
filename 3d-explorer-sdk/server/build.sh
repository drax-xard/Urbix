#!/bin/sh
# server/build.sh
#
# Builds the micro-server against the shipped SDK (static lib).
# Run from 3d-explorer-sdk/:
#   ./server/build.sh
#
# Portable: picks link flags by OS (macOS frameworks vs Linux -ldl/-pthread
# vs Windows MinGW ws2_32). The vendored sdk/lib carries the current host's
# binaries; other targets come from the GitHub Release tarballs
# (sdk/urbix-<ver>-<target>.tar.gz, same layout — swap sdk/lib + sdk/include).
set -e
cd "$(dirname "$0")/.."

OS="$(uname -s 2>/dev/null || echo unknown)"
case "$OS" in
    Darwin*)
        LINK_FLAGS="-framework Security -framework CoreFoundation -lm"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        LINK_FLAGS="-lws2_32 -luserenv"
        ;;
    *)
        LINK_FLAGS="-ldl -lm -pthread"
        ;;
esac

# Windows staticlib is urbix.lib when vendored from the windows tarball.
if [ -f sdk/lib/urbix.lib ] && [ ! -f sdk/lib/liburbix.a ]; then
    SDK_LIB="sdk/lib/urbix.lib"
else
    SDK_LIB="sdk/lib/liburbix.a"
fi

# shellcheck disable=SC2086
cc -O2 -Wall -Wextra -I sdk/include server/serve.c "$SDK_LIB" \
    $LINK_FLAGS \
    -o server/serve

echo "built server/serve ($OS)"
echo "run: ./server/serve --port 8311 --seed 445566 --web server/www"
