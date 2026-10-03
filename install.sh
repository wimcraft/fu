#!/bin/sh
set -eu

repo=${FU_REPO:-wimcraft/fu}
version=${FU_VERSION:-latest}
install_dir=${FU_INSTALL_DIR:-"$HOME/.local/bin"}

case "$(uname -s)" in
    Darwin) ;;
    *)
        printf '%s\n' "fu: prebuilt binaries currently support macOS only" >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    arm64) target=aarch64-apple-darwin ;;
    x86_64) target=x86_64-apple-darwin ;;
    *)
        printf '%s\n' "fu: unsupported architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

case "$version" in
    latest) release_url="https://github.com/$repo/releases/latest/download" ;;
    v*) release_url="https://github.com/$repo/releases/download/$version" ;;
    *) release_url="https://github.com/$repo/releases/download/v$version" ;;
esac

archive="fu-$target.tar.gz"
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/fu-install.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

curl -fsSL "$release_url/$archive" -o "$tmp_dir/$archive"
curl -fsSL "$release_url/SHA256SUMS" -o "$tmp_dir/SHA256SUMS"

expected=$(sed -n "s/^\([0-9a-fA-F][0-9a-fA-F]*\)  $archive$/\1/p" "$tmp_dir/SHA256SUMS")
if [ -z "$expected" ]; then
    printf '%s\n' "fu: checksum for $archive is missing" >&2
    exit 1
fi

actual=$(shasum -a 256 "$tmp_dir/$archive" | awk '{print $1}')
if [ "$actual" != "$expected" ]; then
    printf '%s\n' "fu: checksum verification failed for $archive" >&2
    exit 1
fi

tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"
mkdir -p "$install_dir"
install -m 0755 "$tmp_dir/fu" "$install_dir/fu"
printf '%s\n' "Installed fu to $install_dir/fu"
