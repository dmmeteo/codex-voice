#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    printf 'Usage: %s /path/to/Codex-Voice-macos.zip\n' "$0" >&2
    exit 2
fi
if [ "$(uname -s)" != Darwin ]; then
    printf 'Codex Voice packaging requires macOS.\n' >&2
    exit 1
fi

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
caller_dir=$(pwd -P)
case $1 in
    /*) archive=$1 ;;
    *) archive=$caller_dir/$1 ;;
esac
archive_dir=$(dirname -- "$archive")
mkdir -p "$archive_dir"
archive_dir=$(CDPATH= cd -- "$archive_dir" && pwd -P)
archive=$archive_dir/$(basename -- "$archive")

if ! git -C "$repo_dir" diff --quiet HEAD --; then
    printf 'Commit tracked source changes before packaging.\n' >&2
    exit 1
fi
revision=$(git -C "$repo_dir" rev-parse HEAD)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -n 1)
target=$(rustc -vV | sed -n 's/^host: //p')
case $(uname -m):$target in
    arm64:aarch64-apple-darwin|x86_64:x86_64-apple-darwin) ;;
    *) printf 'Rust host %s does not match this Mac.\n' "$target" >&2; exit 1 ;;
esac
case ${CARGO_TARGET_DIR:-target} in
    /*) target_dir=${CARGO_TARGET_DIR} ;;
    *) target_dir=$repo_dir/${CARGO_TARGET_DIR:-target} ;;
esac

(
    cd "$repo_dir"
    cargo build --locked --release --target "$target" --target-dir "$target_dir"
)
binary=$target_dir/$target/release/codex-voice
if [ "$(lipo -archs "$binary")" != "$(uname -m)" ]; then
    printf 'Release binary does not match this Mac architecture.\n' >&2
    exit 1
fi
linked=$(otool -L "$binary")
printf '%s\n' "$linked" | awk 'NR > 1 { print $1 }' | while IFS= read -r library; do
    case $library in
        /usr/lib/*|/System/Library/*) ;;
        *) printf 'Non-system dynamic library: %s\n' "$library" >&2; exit 1 ;;
    esac
done

stage=$(mktemp -d "${TMPDIR:-/tmp}/codex-voice.XXXXXX")
temp_dir=$(mktemp -d "$archive_dir/.codex-voice.XXXXXX")
temp_archive=$temp_dir/package.zip
temp_sha=$temp_dir/package.sha256
cleanup() {
    rm -rf "$stage" "$temp_dir"
}
trap cleanup 0
trap 'exit 1' 1 2 3 15

cp "$binary" "$stage/codex-voice"
cp "$repo_dir/packaging/voice/start.command" "$stage/start.command"
cp "$repo_dir/packaging/voice/README.md" "$stage/README.md"
cp "$repo_dir/docs/voice.md" "$stage/VOICE.md"
cp "$repo_dir/LICENSE-MIT" "$stage/LICENSE-MIT"
cp "$repo_dir/THIRD_PARTY_NOTICES.md" "$stage/THIRD_PARTY_NOTICES.md"
chmod 755 "$stage/codex-voice" "$stage/start.command"
binary_sha=$(shasum -a 256 "$stage/codex-voice" | awk '{ print $1 }')
cat >> "$stage/README.md" <<EOF

## Build receipt

- Source revision: \`$revision\`
- Crate version: \`$version\`
- Native target: \`$target\`
- Binary SHA-256: \`$binary_sha\`

The revision identifies the local source build. The crate version alone does
not identify this local package.
EOF

(
    cd "$stage"
    zip -q -X "$temp_archive" codex-voice start.command LICENSE-MIT README.md VOICE.md
)
unzip -tq "$temp_archive" >/dev/null
entries=$(unzip -Z -1 "$temp_archive" | LC_ALL=C sort)
expected=$(printf '%s\n' codex-voice start.command LICENSE-MIT README.md VOICE.md | LC_ALL=C sort)
if [ "$entries" != "$expected" ]; then
    printf 'Archive contents differ from the package allowlist.\n' >&2
    exit 1
fi
archive_sha=$(shasum -a 256 "$temp_archive" | awk '{ print $1 }')
printf '%s  %s\n' "$archive_sha" "$(basename -- "$archive")" > "$temp_sha"
mv -f "$temp_archive" "$archive"
mv -f "$temp_sha" "$archive.sha256"
printf 'Wrote %s\nSHA-256 %s\n' "$archive" "$archive_sha"
