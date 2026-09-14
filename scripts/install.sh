#!/bin/sh
# The release pipeline replaces this marker with the archive's exact version.
# Downloaded installers stay pinned even when a newer release is published.
main() {
    set -eu

    version='@VERSION@'
    install_dir=${HOME:+"$HOME/.local/bin"}
    work_dir=
    staged_binary=

    fail() { printf 't installer: %s\n' "$*" >&2; exit 1; }
    cleanup() {
        if [ -n "$staged_binary" ]; then rm -f "$staged_binary"; fi
        if [ -n "$work_dir" ]; then rm -rf "$work_dir"; fi
    }
    trap cleanup 0
    trap 'exit 1' HUP INT TERM

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version)
                [ "$#" -ge 2 ] || fail '--version requires a value'
                version=$2; shift 2 ;;
            --install-dir)
                [ "$#" -ge 2 ] || fail '--install-dir requires a value'
                install_dir=$2; shift 2 ;;
            -h|--help)
                printf '%s\n' 'Usage: sh install.sh [--version v0.1.0] [--install-dir PATH]'
                return 0 ;;
            *) fail "unknown option: $1" ;;
        esac
    done

    # Stable SemVer only; also prevents path traversal in download URLs.
    printf '%s\n' "$version" | grep -Eq '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
        || fail 'expected a release version such as v0.1.0 (source installers require --version)'
    [ -n "$install_dir" ] || fail 'set HOME or pass --install-dir PATH'
    case "$install_dir" in
        /*) ;;
        *) install_dir=$PWD/$install_dir ;;
    esac

    for tool in curl tar mktemp install mv; do
        command -v "$tool" >/dev/null 2>&1 || fail "required tool not found: $tool"
    done
    if command -v sha256sum >/dev/null 2>&1; then
        checksum_tool=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum_tool=shasum
    else
        fail 'required tool not found: sha256sum or shasum'
    fi

    os=$(uname -s)
    arch=$(uname -m)
    if [ "$os" = Darwin ] && [ "$arch" = x86_64 ]; then
        if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
            arch=arm64
        fi
    fi
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail "unsupported architecture: $arch" ;;
    esac
    case "$os" in
        Linux) target=$arch-unknown-linux-musl ;;
        Darwin) target=$arch-apple-darwin ;;
        *) fail "unsupported operating system: $os" ;;
    esac

    archive=t-$version-$target.tar.gz
    base_url=https://github.com/sdrshnv/t/releases/download/$version
    work_dir=$(mktemp -d "${TMPDIR:-/tmp}/t-install.XXXXXXXX")
    printf 'Downloading t %s for %s...\n' "$version" "$target"
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
        "$base_url/$archive" -o "$work_dir/$archive" || fail 'binary download failed'
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
        "$base_url/SHA256SUMS" -o "$work_dir/SHA256SUMS" || fail 'checksum download failed'

    expected=$(awk -v name="$archive" '$2 == name {print $1}' "$work_dir/SHA256SUMS")
    [ "${#expected}" -eq 64 ] || fail 'missing or invalid archive checksum'
    case "$expected" in *[!0-9a-f]*) fail 'invalid archive checksum' ;; esac
    if [ "$checksum_tool" = sha256sum ]; then
        actual=$(sha256sum "$work_dir/$archive")
    else
        actual=$(shasum -a 256 "$work_dir/$archive")
    fi
    actual=${actual%% *}
    [ "$actual" = "$expected" ] || fail 'checksum mismatch; existing installation was not changed'

    tar -xzf "$work_dir/$archive" -C "$work_dir" t || fail 'could not extract binary'
    if [ ! -f "$work_dir/t" ] || [ -L "$work_dir/t" ]; then
        fail 'archive does not contain a regular binary'
    fi
    mkdir -p "$install_dir" || fail "could not create $install_dir"
    [ ! -d "$install_dir/t" ] || fail "$install_dir/t is a directory"
    staged_binary=$(mktemp "$install_dir/.t.XXXXXXXX")
    install -m 755 "$work_dir/t" "$staged_binary" || fail 'could not stage binary'
    mv -f "$staged_binary" "$install_dir/t" || fail 'could not install binary'
    staged_binary=
    printf 'Installed t %s to %s/t\n' "$version" "$install_dir"
    case ":${PATH:-}:" in
        *":$install_dir:"*) ;;
        *)
            printf 'Add this directory to PATH in your shell configuration: %s\n' "$install_dir"
            # shellcheck disable=SC2016 # Print the shell configuration literally.
            printf '%s\n' 'For the default directory in bash/zsh: export PATH="$HOME/.local/bin:$PATH"'
            ;;
    esac
}

main "$@"
