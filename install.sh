#!/bin/sh
# Installs sp, Session Player's command line, on Linux:
#
#   curl -fsSL https://raw.githubusercontent.com/danmactough/session-player/main/install.sh | sh
#
# It downloads the sp built for this machine from the latest GitHub release and
# puts it at ~/.local/bin/sp. Run it again to update. On a Mac, sp comes inside
# the app instead.
#
# SP_VERSION=v0.17.0 installs that release instead of the latest.

set -eu

REPO="danmactough/session-player"
DIR="$HOME/.local/bin"

die() {
  echo "Error: $*" >&2
  exit 1
}

case "$(uname -s)" in
  Linux) ;;
  Darwin) die "on a Mac, sp comes inside the app. Install Session Player, then choose File > Install Command Line Tool" ;;
  *) die "there is no sp for $(uname -s)" ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) die "there is no sp for $(uname -m) processors" ;;
esac

# Statically linked, so one file runs on any distribution.
FILE="sp-$ARCH-unknown-linux-musl.tar.gz"
if [ -n "${SP_VERSION:-}" ]; then
  BASE="https://github.com/$REPO/releases/download/$SP_VERSION"
else
  BASE="https://github.com/$REPO/releases/latest/download"
fi
# For testing this script against files that aren't a release.
BASE="${SP_INSTALL_BASE_URL:-$BASE}"

command -v curl >/dev/null || die "curl is not installed"
command -v tar >/dev/null || die "tar is not installed"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $BASE/$FILE"
curl -fsSL "$BASE/$FILE" -o "$TMP/$FILE" || die "couldn't download $FILE. Does that release have Linux files yet?"
curl -fsSL "$BASE/$FILE.sha256" -o "$TMP/$FILE.sha256" || die "couldn't download the checksum for $FILE"

if command -v sha256sum >/dev/null; then
  (cd "$TMP" && sha256sum -c "$FILE.sha256" >/dev/null) || die "$FILE doesn't match its checksum; nothing was installed"
else
  echo "sha256sum is not installed, so the download wasn't checked against its checksum."
fi

tar -xzf "$TMP/$FILE" -C "$TMP"
[ -f "$TMP/sp" ] || die "$FILE has no sp in it"
chmod +x "$TMP/sp"

mkdir -p "$DIR"
# Moved into place in one step, so an sp that is running isn't written over.
mv "$TMP/sp" "$DIR/.sp-installing"
mv "$DIR/.sp-installing" "$DIR/sp"

echo "Installed $("$DIR/sp" --version) at $DIR/sp"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *) echo "$DIR isn't on your PATH. Add it to run sp by name, for example with this line in ~/.profile:"
     echo "  export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
esac
echo "Next: run 'sp index' to read this machine's sessions."
