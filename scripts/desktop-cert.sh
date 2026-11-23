#!/usr/bin/env bash
# One-time: create the self-signed "Recapz Local" code-signing certificate in the login keychain.
# Every build signed with it has the same identity, so the mic and audio-capture grants survive an update.
# Do not delete it: a new certificate counts as a different app and macOS asks for the grants again.
set -euo pipefail

NAME="Recapz Local"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-certificate -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "\"$NAME\" already exists in the login keychain."
  exit 0
fi

dir=$(mktemp -d)
trap 'rm -f "${dir:?}"/*; rmdir "$dir"' EXIT
pass=$(/usr/bin/openssl rand -hex 16)

# Ten years, code-signing usage only (codesign refuses a certificate without it).
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=$NAME" \
  -addext "keyUsage=critical,digitalSignature" -addext "extendedKeyUsage=critical,codeSigning" \
  -keyout "$dir/key.pem" -out "$dir/cert.pem" 2>/dev/null
# /usr/bin/openssl (LibreSSL) writes a PKCS#12 that `security` can read; Homebrew OpenSSL 3+ does not by default.
/usr/bin/openssl pkcs12 -export -inkey "$dir/key.pem" -in "$dir/cert.pem" -passout "pass:$pass" -out "$dir/id.p12"
security import "$dir/id.p12" -k "$KEYCHAIN" -P "$pass" -T /usr/bin/codesign

echo "Created \"$NAME\". It shows as untrusted in Keychain Access; that is expected and codesign still uses it."
