#!/usr/bin/env bash
# Make the signing key for LightCraft for Android releases, once.
#
# Android installs an update only when it is signed with the same key as the installed app, so
# the in-app updater needs every release signed with one key. This makes that key and prints the
# three values to store as repository secrets (Settings ▸ Secrets and variables ▸ Actions):
# ANDROID_KEYSTORE_BASE64, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS.
#
#   tools/android/make-release-key.sh [keystore-path]
#
# Keep the keystore and its password somewhere safe (a password manager): without them no later
# release can update the installed app. Never commit it.
set -euo pipefail

KS="${1:-lightcraft-release.keystore}"
ALIAS=lightcraft
if [[ -e "$KS" ]]; then
    echo "$KS already exists; not replacing it." >&2
    exit 1
fi
PASS="$(head -c 32 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 32)"
keytool -genkeypair -keystore "$KS" -storetype PKCS12 -storepass "$PASS" -keypass "$PASS" -alias "$ALIAS" \
    -keyalg RSA -keysize 4096 -validity 36500 -dname "CN=LightCraft for Android" >/dev/null 2>&1
chmod 600 "$KS"
echo "Keystore: $KS (keep it, and never commit it)"
echo
echo "ANDROID_KEY_ALIAS:"
echo "$ALIAS"
echo
echo "ANDROID_KEYSTORE_PASSWORD:"
echo "$PASS"
echo
echo "ANDROID_KEYSTORE_BASE64:"
base64 -w0 "$KS"
echo
