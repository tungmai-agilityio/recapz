# Document view

Status: **agreed with the maintainer (2026-11-27); server built (#700, merged), page built in #701.** Not seen against a real server: unverified. The 1 MB browser threshold is unverified.
Intent: none (two tickets, milestone M23).

## Problem

A member can upload Markdown or TXT documents for Ask, but cannot open or download one again. The My documents page lists, toggles and deletes only.

## Decisions

- **Local disk stays:** documents remain under `<data_dir>/tenants/<workspace>/documents/<id>.md|txt`. The object store holds only recordings and the offsite backup.
- **Uploader only:** another member, an admin, the same user in another workspace, a deleted document and an invalid id all answer 404.
- **Plain text reader:** the page shows the first 1 MB as preformatted wrapped text, with no Markdown renderer and no new dependency. When the file is longer, a notice points to Download.
- **Download is the original bytes** as an attachment with a fixed text type, `nosniff` and `no-store`, so a browser never runs or caches it.
- **Ask citations:** a document card in an Ask answer links to the document page.
- **Deferred:** "which Ask answers cite this document" waits for Ask history (`docs/specs/ask-history.md`).

## Out of scope

Editing, Markdown rendering, search inside the page, sharing a document with other members.

## Acceptance

- [ ] The uploader opens a document from the My documents table and reads it; Download returns the exact bytes.
- [ ] Nobody else can read it (404 for each case above).
- [ ] A file longer than 1 MB shows the first 1 MB and a notice; exactly 1 MB shows no notice.
- [ ] Delete from the page returns to the list.

## Tickets

| # | Title | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| #700 | feat(server): view and download a document | server | none | `crates/server/src/routes/documents.rs`, `routes.rs`, `crates/core/src/document.rs`, tests, `docs/spec.md` | small to medium | `cargo test --workspace` |
| #701 | feat(web): document page to read and download a document | web | #700 | `web/src/pages/`, `web/src/api.ts`, `docs/ui-parity.md` | small to medium | `pnpm -C web test` |

## Open questions

None.
