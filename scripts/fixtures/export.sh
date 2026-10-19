#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
cd "$repo_root"
# F1 is the committed browser shell export. F3 and F8 are also populated
# in the fixture catalog; skeleton fixtures cannot produce meaningful counts.
cargo xtask fixture-export F1 --shell --out "$temporary/F1.json"
mkdir -p apps/desktop/ui/fixtures
cp "$temporary/F1.json" apps/desktop/ui/fixtures/F1.json
