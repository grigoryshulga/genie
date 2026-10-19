#!/usr/bin/env bash
# Install the same Linux x64 Pi-Bolt build as Docker, without changing the host's pi.
set -euo pipefail

destination="${1:?usage: bash .github/scripts/install-pi-bolt.sh <installation-directory>}"
if [[ "$(uname -sm)" != "Linux x86_64" ]]; then
  echo 'The Docker-pinned Pi-Bolt package supports Linux x86_64 only.' >&2
  exit 1
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
version="$(sed -n 's/^ARG PI_BOLT_VERSION=//p' "$repo/docker/Dockerfile")"
checksum="$(sed -n 's/^ARG PI_BOLT_SHA256=//p' "$repo/docker/Dockerfile")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && "$checksum" =~ ^[a-f0-9]{64}$ ]] || {
  echo 'Missing or invalid Pi-Bolt version/checksum in docker/Dockerfile.' >&2
  exit 1
}

download="$(mktemp -d)"
trap 'rm -rf "$download"' EXIT
curl --fail --silent --show-error --location --retry 3 \
  "https://registry.npmjs.org/pi-bolt-linux-x64/-/pi-bolt-linux-x64-${version}.tgz" -o "$download/bolt.tgz"
echo "$checksum  $download/bolt.tgz" | sha256sum -c -
tar xzf "$download/bolt.tgz" -C "$download" package/pi-bolt-linux-x64.tar.xz
mkdir -p "$destination"
tar xJf "$download/package/pi-bolt-linux-x64.tar.xz" -C "$destination" --strip-components=1
"$destination/pi" --version
