# Ask history

Status: **agreed with the maintainer (2026-11-27); built in #696 to #698 (the tickets' PRs; the first two need the maintainer to merge). Not yet seen against a real server: unverified. Nothing here is verified in a running app.
Intent: #695. Docs ticket: #699.

## Problem

Ask questions and answers vanish on a page reload. `web/src/pages/Ask.tsx` keeps the turns in React state, and
`POST /api/ask` stores only an `llm_usage` row. A member who asked a good question yesterday cannot get it back.

## Decisions

- **Flat list per member:** the Ask page keeps one list, as it shows today. No separate chats. Earlier turns are not sent to the model, so every question is still answered on its own.
- **Private to the member:** admins cannot read it, in the API or the database queries Recapz ships. Each workspace keeps its own, in its tenant SQLite.
- **Saved when the answer completes:** the server saves a turn (question, answer, citation cards) when the stream finishes, even if the browser went away. A failed answer is not saved. The `done` event carries the turn `id` so the page can delete it without a reload.
- **Access loss hides, purge deletes:** a turn that cites a meeting the member can no longer see is hidden from the list (the list applies `db::MEETING_VISIBLE`), not deleted, so it returns if access returns. A turn citing a meeting, a participant's speech or a document that is purged is hard-deleted in the same transaction as the purge. Removing a member deletes their turns.
- **Cap:** about 200 turns per member. Saving a new turn prunes the oldest beyond the cap.
- **Delete one, clear all:** the member can delete one turn or clear the whole history. Clear asks for confirmation.
- **No question text in logs:** logs carry IDs only.

## Out of scope

- Conversation memory (sending earlier turns to the model) and named chats.
- Searching within history, export, sharing a turn.
- Admin or audit access to members' history.

## Acceptance

- [ ] After a reload the Ask page shows the member's earlier turns with their citation cards.
- [ ] A member never receives, deletes or counts another member's turns; an admin gets the same answer as any other member.
- [ ] A turn citing a meeting the member can no longer see is not listed.
- [ ] Purging a meeting, a participant's speech or a document, and removing a member, hard-deletes the matching turns.
- [ ] Delete one and Clear history work from the page; Clear asks first. Matches the prototype in both skins and on mobile.
- [ ] Logs contain no question or answer text.

## API

- `POST /api/ask`: unchanged request; `done` gains `id` (the saved turn), and the turn is saved when the answer completes.
- `GET /api/ask/history`: the member's latest turns, oldest first: `id`, `question`, `answer`, `citations`, `created_at`. Hides turns citing meetings the member cannot see.
- `DELETE /api/ask/history/{id}`: one of the member's own turns; 404 for another member's or a missing turn.
- `DELETE /api/ask/history`: clears the member's turns.
- The two deletes need CSRF.

## Storage

Table `ask_turns` in the tenant database, migration `tenant_010` (additive): `id`, `member_id`, `question`, `answer`,
`citations_json`, `created_at`, with an index on (`member_id`, `created_at`). Citations are stored as the cards the
stream sent, so a restored turn looks as it did.

## UI

Prototype: https://claude.ai/artifact/KxQaWLV4L9kusPFz1eonzH (Ask, updated 2026-11-27): a delete button on each turn
and a Clear history button with an inline confirmation. The new rows are in `docs/ui-parity.md` (S4 Ask), unticked
until #698.

## Tickets

| # | Title | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| #699 | docs(spec): plan Ask history | docs | none | `docs/specs/ask-history.md`, `docs/spec.md`, `docs/ui-parity.md` | small | docs only |
| #696 | feat(core): store Ask history and delete it with purged data | core | #699 | `crates/core/migrations/tenant_010.sql`, `crates/core/src/ask_history.rs`, `purge.rs`, `members.rs`, `lib.rs` | small to medium | `cargo test --workspace` |
| #697 | feat(server): save Ask answers and serve the member's history | server | #696 | `crates/server/src/routes/ask.rs`, `routes.rs`, `docs/spec.md` | small to medium | `cargo test --workspace` |
| #698 | feat(web): show Ask history after a reload | web | #697 | `web/src/pages/Ask.tsx`, `Ask.test.tsx`, `web/src/api.ts`, `docs/ui-parity.md` | small to medium | `pnpm -C web test` |

## Open questions

None.
