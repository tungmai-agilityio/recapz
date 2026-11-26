# Desktop auto-update with a local signing key

Status: **decided (2026-11-25), implemented (#369 to #372); end-to-end test not recorded as run.**
Spec: `docs/specs/desktop-app.md`, "Auto-update".

## Decision

The macOS app updates itself with the Tauri updater. The manifest `latest.json` and the signed `Recapz.app.tar.gz` are
GitHub release assets of this public repository. The update private key and its password exist only on the
maintainer's Mac and in a password manager: never in the repository, chat, logs or CI. The public key is committed in
`desktop/src-tauri/tauri.conf.json`.

## Why

Rebuilding and reinstalling the `.dmg` by hand for every shell change was the pain. There is no macOS CI (the free
plan bills macOS minutes at 10x), so a CI-held key would need a macOS runner and a repository secret for no gain. A
public repository needs no token in the app.

## Consequences

- Losing the key means installed apps can no longer be updated; every member installs again by hand with a new key.
- The newest release must always carry a `latest.json`, or the check gets a 404. The repository has no GitHub release
  yet (checked 2026-11-25), so the check currently fails quietly.
- If the repository becomes private, move the endpoint to the Recapz server; never ship a GitHub token in the app.
- No rollback of a broken update.
- Web changes never need an app update: the window loads the server's UI.

## Unverified

No quarantine attribute after an update, grants surviving an update, overwriting `/Applications` as a non-admin, and the
full build with the key passed as content (#372) are not recorded as verified (#426).
