#!/bin/sh
set -eu
test "$1" = rc
shift
exec /usr/bin/llvm-rc "$@"
