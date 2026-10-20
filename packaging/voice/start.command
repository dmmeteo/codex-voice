#!/bin/sh
set -eu

package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
exec "$package_dir/codex-voice" serve --no-api-key "$@"
