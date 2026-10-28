#!/usr/bin/env bash
# Builds a signed, notarized Session Player .dmg and publishes it as a GitHub
# release tagged v<version>, using the version in src-tauri/tauri.conf.json.
#
# Usage: ./build-and-publish.sh [--no-publish]
#   --no-publish   build, notarize and verify only; skip the push and release
#
# One-time setup:
#   1. A "Developer ID Application" certificate in your login keychain
#      (Xcode > Settings > Accounts > Manage Certificates).
#   2. An app-specific password (account.apple.com) saved in the keychain:
#        security add-generic-password -a <apple-id-email> -s session-player-notary -w
#   3. gh authenticated for this repository (gh auth status).
#
# The Apple ID comes from that keychain item and the Team ID from the
# certificate, so neither is stored in this repository. Override with
# APPLE_SIGNING_IDENTITY, APPLE_ID or APPLE_TEAM_ID if needed.

set -euo pipefail

KEYCHAIN_SERVICE="session-player-notary"
PUBLISH=1

for arg in "$@"; do
  case "$arg" in
    --no-publish) PUBLISH=0 ;;
    -h | --help)
      sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "Unknown option: $arg" >&2
      exit 1
      ;;
  esac
done

cd "$(dirname "$0")"

step() { printf '\n==> %s\n' "$*"; }
die() {
  echo "Error: $*" >&2
  exit 1
}

for tool in jq gh xcrun security npx; do
  command -v "$tool" >/dev/null || die "$tool is not installed"
done

VERSION=$(jq -r .version src-tauri/tauri.conf.json)
TAG="v$VERSION"
DMG="src-tauri/target/release/bundle/dmg/Session Player_${VERSION}_aarch64.dmg"

step "Checking release $TAG"
# Publishing must not ship code that isn't committed, or reuse a version.
[ -z "$(git status --porcelain)" ] || die "the working tree has uncommitted changes"
if [ "$PUBLISH" = 1 ]; then
  [ "$(git branch --show-current)" = "main" ] || die "publish from main"
  if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || gh release view "$TAG" >/dev/null 2>&1; then
    die "$TAG already exists; bump version in src-tauri/tauri.conf.json"
  fi
fi

step "Reading signing credentials"
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  APPLE_SIGNING_IDENTITY=$(security find-identity -v -p codesigning |
    sed -n 's/.*"\(Developer ID Application: .*\)".*/\1/p' | head -1)
fi
[ -n "$APPLE_SIGNING_IDENTITY" ] || die "no Developer ID Application certificate found in the keychain"
# The Team ID is the code in parentheses at the end of the certificate name.
APPLE_TEAM_ID=${APPLE_TEAM_ID:-$(printf '%s' "$APPLE_SIGNING_IDENTITY" | sed -n 's/.*(\([A-Z0-9]*\))$/\1/p')}
APPLE_ID=${APPLE_ID:-$(security find-generic-password -s "$KEYCHAIN_SERVICE" 2>/dev/null |
  sed -n 's/.*"acct"<blob>="\(.*\)"/\1/p')}
[ -n "$APPLE_ID" ] || die "no keychain item named $KEYCHAIN_SERVICE (see setup in this script)"
APPLE_PASSWORD=$(security find-generic-password -s "$KEYCHAIN_SERVICE" -w)
echo "Signing as: $APPLE_SIGNING_IDENTITY"
export APPLE_SIGNING_IDENTITY APPLE_ID APPLE_TEAM_ID APPLE_PASSWORD

step "Building, signing and notarizing the app"
npm ci --ignore-scripts
npx tauri build --bundles dmg
[ -f "$DMG" ] || die "expected $DMG after the build"

# Tauri notarizes the .app inside the disk image; the disk image itself needs
# its own notarization so macOS opens a downloaded copy without a warning.
step "Notarizing the disk image"
xcrun notarytool submit "$DMG" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" --wait
xcrun stapler staple "$DMG"

step "Verifying"
spctl -a -vvv -t open --context context:primary-signature "$DMG"

if [ "$PUBLISH" = 0 ]; then
  step "Done (not published): $DMG"
  exit 0
fi

step "Publishing $TAG"
git push origin main
gh release create "$TAG" "$DMG" --target main --title "Session Player $VERSION" --generate-notes

step "Published: $(gh release view "$TAG" --json url -q .url)"
