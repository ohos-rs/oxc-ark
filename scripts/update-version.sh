#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
version="${1:-${npm_new_version:-${npm_package_version:-}}}"

if [[ -z "$version" ]]; then
  version="$(awk -F'"' '/"version"[[:space:]]*:/ { print $4; exit }' "$repo_root/npm/oxk/package.json")"
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: scripts/update-version.sh <semver>" >&2
  exit 1
fi

echo "Updating @ohos-rs/oxk to $version"

export VERSION="$version"

perl -0pi -e 's/("version"\s*:\s*")[^"]+(")/$1$ENV{VERSION}$2/;' "$repo_root/npm/oxk/package.json"
perl -0pi -e 's/(\[package\][\s\S]*?\nversion\s*=\s*")[^"]+(")/$1$ENV{VERSION}$2/;' "$repo_root/crates/oxk/Cargo.toml"
perl -0pi -e 's/(\[\[package\]\]\nname = "oxk"\nversion = ")[^"]+(")/$1$ENV{VERSION}$2/;' "$repo_root/Cargo.lock"

(
  cd "$repo_root"
  pnpm --filter @ohos-rs/oxk exec napi version
  bash "$repo_root/scripts/verify-release.sh"
)
