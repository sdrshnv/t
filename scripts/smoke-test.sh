#!/bin/sh
set -eu
binary=$1
version=$2
work_dir=$(mktemp -d)
trap 'rm -rf "$work_dir"' 0
trap 'exit 1' HUP INT TERM
export XDG_DATA_HOME="$work_dir"
[ "$("$binary" --version)" = "t $version" ]
"$binary" --help >/dev/null
"$binary" add 1 'Release smoke test' >/dev/null
"$binary" ls | grep -F 'Release smoke test'
"$binary" 'done' 1 >/dev/null
[ -z "$("$binary")" ]
