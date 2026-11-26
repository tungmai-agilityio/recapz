# Workspaces

Status: **agreed with the maintainer (2026-11-25)**
Intent: #428

## Problem

Recapz runs one team per server. The storage is already split by workspace (one `recapz.sqlite` per tenant,
object keys prefixed by tenant, background loops that visit every tenant), but the people side is missing. An
admin cannot invite, remove or change the role of anyone from the app: the only invite ever created is the
bootstrap admin's, so every other member is added by editing `control.sqlite`. One email can belong to only one
workspace (`users.email UNIQUE`, one `users.tenant_id`, `invites.email` primary key), and a second workspace can
only be made by hand. The notetaker's concurrent calls and the monthly cost cap are shared by the whole server,
so one busy workspace can take every notetaker slot.

## Decisions

- **One person, several workspaces:** a `memberships (user_id, tenant_id, role)` table in `control.sqlite`
  replaces `users.tenant_id` and `users.role` as the source of truth. Both old columns stay (migrations are
  additive); `users.tenant_id` becomes "the workspace used last", and `users.role` is no longer read.
- **The workspace lives in the session, not the URL:** `sessions` gains a `tenant_id` column, and
  `db::user_for_session` returns the session's workspace and the role from its membership. Every handler already
  reads `u.tenant_id` and `u.role`, so they keep working unchanged. No subdomains: the desktop app is built for one
  `BASE_URL`, the CSRF check compares `Origin` with `base_url`, and Google sign-in has one redirect URL.
- **Sign-in picks the workspace used last;** switching sets `sessions.tenant_id` and `users.tenant_id`. A
  session whose membership is gone signs the person out (401), as a deleted session does today.
- **Invites are a link the admin copies, no email provider:** an invite is a row `(email, tenant_id, role)` in a
  new `workspace_invites` table, valid 14 days. The link is the plain sign-in page (`{base_url}/`) and carries no
  token: signing in with Google as that email accepts every pending invite for it, so a leaked link grants
  nothing. A person already signed in sees new workspaces in the switcher, because listing workspaces accepts
  pending invites first.
- **Roles stay `admin` and `member`;** the server refuses to remove or demote the last admin of a workspace.
- **Removing a member** (admin only, never yourself): their meetings move to the admin who removed them, keeping
  each meeting's `share`; their personal documents and their calendar connection are hard-deleted (no one else
  can see them, and the calendar token is theirs); their upcoming calendar bookings are removed; their sessions in
  that workspace end; the tenant `members` row is kept with `status = 'removed'` so speaker labels and past owners
  still resolve. They can be invited again.
- **The tenant `members` row follows the membership:** `ensure_member` becomes an upsert of name and role, so a
  role change is visible inside the tenant on the next request.
- **The server owner creates workspaces from the command line:** `recapz workspace-create <name> <admin email>`
  makes the tenant, its database and an admin invite; `recapz workspace-list` prints id, name, members and this
  month's cost. Self-service sign-up is out of scope.
- **Notetaker slots are shared fairly:** `max_concurrent_calls` stays a server-wide limit, but due calls are
  claimed one per workspace in turn (round robin), so a workspace with many due meetings cannot take every slot
  while another one waits.
- **Cost caps per workspace and for the server:** `monthly_cap_usd` in the config stays the default cap for each
  workspace; the server owner can set a different cap for one workspace (`recapz workspace-cap <id> <usd>`,
  stored on `tenants`). A new `server_monthly_cap_usd` (0 = off) pauses paid jobs in every workspace once the sum
  of all workspaces reaches it. Workspace admins cannot raise caps: the provider keys and the bill are the server
  owner's.

## Out of scope

- Self-service workspace sign-up, deleting or exporting a whole workspace, billing.
- Joining a workspace by Google domain, SAML or other single sign-on.
- A notetaker Google account per workspace; workspaces share the server's notetaker, which already uses each
  workspace's notetaker name and join message.
- Subdomains or workspace ids in URLs; a desktop app that talks to several servers.
- Renaming a workspace from the app (the command line sets the name).
- A workspace default for meeting sharing; sharing one meeting is #407.

## Acceptance

- [ ] An admin invites an email with a role from the Members screen, copies the link, and that person signs in
      with Google and lands in the workspace with that role.
- [ ] A person who belongs to two workspaces switches from the account menu, and every screen (meetings, Ask,
      documents, live view, Recording & privacy, usage) shows only the current workspace's data.
- [ ] Any meeting, document or calendar route called with an id from another workspace answers 404, including the
      live view and the recording socket.
- [ ] Removing a member ends their sessions there, moves their meetings to the admin, and hard-deletes their
      documents and calendar connection; the last admin cannot be removed or demoted.
- [ ] Existing servers upgrade with no manual step: every user keeps their workspace, role and sessions.
- [ ] With a slot limit of 2, workspace A with five due meetings and workspace B with one get one call each.
- [ ] A workspace over its cap pauses only its own paid jobs; the server cap pauses all of them.

## Tickets

One ticket is one PR (about 300 source lines small, 600 medium). Order is by dependency. `touches` lists the
paths the ticket changes. Testing scope follows "Testing & Verification" in `CLAUDE.md`.

| # | Title (`type(scope): summary`) | Area | Blocked by | Touches | Size | Testing scope |
|---|---|---|---|---|---|---|
| 1 (#429) | feat(core): add workspace memberships and invites | core, auth | none | `crates/core/migrations/control_002.sql`, `crates/core/src/db.rs` | medium | `cargo test --workspace` (auth, migrations) |
| 2 (#430) | feat(core): manage the members of a workspace | core | 1 | `crates/core/src/members.rs`, `crates/core/src/lib.rs` | medium | `cargo test --workspace` (meeting access, user-data deletion) |
| 3 (#431) | feat(server): add the members API | server, auth | 2 | `crates/server/src/routes.rs`, `crates/server/src/routes/members.rs`, `crates/server/tests/members.rs` | medium | `cargo test --workspace` (auth, shared API) |
| 4 (#432) | feat(web): add the Members screen | web | 3 | `web/src/pages/Members.tsx`, `web/src/pages/Members.test.tsx`, `web/src/Shell.tsx`, `web/src/App.tsx`, `web/src/api.ts`, `docs/ui-parity.md` | medium | `pnpm -C web test`, lint, browser flow |
| 5 (#433) | feat(server): switch workspaces within a session | server, auth | 1, 3 | `crates/server/src/routes.rs`, `crates/server/src/routes/workspaces.rs`, `crates/server/tests/workspaces.rs` | small | `cargo test --workspace` (sessions, shared API) |
| 6 (#434) | test(server): keep every meeting route inside its workspace | server | 5 | `crates/server/tests/isolation.rs` | small | `cargo test --workspace` (meeting access) |
| 7 (#435) | feat(web): switch workspace from the account menu | web | 4, 5 | `web/src/Shell.tsx`, `web/src/App.test.tsx`, `web/src/api.ts`, `docs/spec.md`, `docs/ui-parity.md` | small | `pnpm -C web test`, lint, browser flow |
| 8 (#436) | feat(server): create and list workspaces from the command line | server, core | 1 | `crates/server/src/cli.rs`, `crates/core/src/db.rs`, `docs/deploy.md` | small | `cargo test -p recapz-core -p recapz-server` |
| 9 (#437) | feat(notetaker): share notetaker slots fairly between workspaces | notetaker, server | none | `crates/server/src/notetaker_loop.rs` | small | `cargo test -p recapz-server` |
| 10 (#438) | feat(jobs): cap cost per workspace and for the whole server | jobs, core, server | 8 | `crates/core/migrations/control_003.sql`, `crates/core/src/usage.rs`, `crates/core/src/db.rs`, `crates/server/src/runner.rs`, `crates/server/src/config.rs`, `crates/server/src/cli.rs`, `config.example.toml` | medium | `cargo test --workspace` (migrations) |

1. `control_002.sql`: `memberships (user_id, tenant_id, role, created_at, PRIMARY KEY (user_id, tenant_id))`,
   `workspace_invites (email, tenant_id, role, invited_by, created_at, PRIMARY KEY (email, tenant_id))`,
   `ALTER TABLE sessions ADD COLUMN tenant_id`; backfill memberships from `users` and invites from `invites`.
   `user_for_sign_in` accepts every unexpired invite for the email and picks `users.tenant_id` if it is still a
   membership, else the oldest one; `create_session` stores the workspace; `user_for_session` returns the
   session's workspace and the membership's role, falling back to `users.tenant_id` for sessions made before the
   migration, and `None` without a membership. `bootstrap_admin` writes the new invite table. Tests: an upgraded
   database keeps every user's workspace, role and session; a two-workspace user gets the workspace used last;
   an invite older than 14 days is ignored. Touches sessions: critical by `CLAUDE.md`, gets `human-merge`.
2. `members.rs` on the control and tenant connections: `list` (members and pending invites, with meeting counts),
   `invite` (refuses an existing member), `set_role` and `remove` with the last-admin guard, and `remove` doing
   what Decisions say in one transaction per database, reusing `purge::purge_document` for documents. Tests assert
   exact rows left in both databases and that removing the guard makes them fail.
3. `GET /api/members`, `POST /api/members/invites` (email, role; returns the link), `DELETE
   /api/members/invites/{email}`, `PATCH /api/members/{id}` (role), `DELETE /api/members/{id}`. Admin only (403
   otherwise), CSRF as every route; `ensure_member` becomes an upsert of name and role. Tests per route for admin,
   member, last admin and self-removal.
4. Members screen from the prototype's S7 rows in `docs/ui-parity.md`: table with face, name, email, role select,
   meeting count, pending badge, Remove or Cancel invite with inline confirm, an invite form and a Copy link
   button. Rail item under Workspace and "Manage members" in the account menu, for admins only; members never
   request the API. Both skins and 390 px.
5. `GET /api/workspaces` (accepts pending invites, then lists id, name, role and which one is current), `POST
   /api/workspaces/{id}/switch` (403 without a membership; sets `sessions.tenant_id` and `users.tenant_id`), and
   `workspace { id, name }` in `/api/me`. Touches sessions: critical, gets `human-merge`.
6. One user in two workspaces; for every route with a meeting, document, calendar event or speaker id, including
   `live`, `record` and `live-ask`, a call with the other workspace's id answers 404 and changes nothing. Fixes
   any leak it finds in the same PR (escalate if the fix is more than a few lines).
7. Account menu lists the workspaces when there are two or more, marks the current one, and switching reloads
   the app on the meetings list. The rail shows the workspace name. Amends `docs/spec.md` ("one team on one
   server", Members, API).
8. `workspace-create` makes the tenant row, runs its migrations and adds an admin invite, refusing a duplicate
   name; `workspace-list` prints id, name, member count and this month's cost. `docs/deploy.md` gets the commands.
9. `due_calls` claims one call per workspace in turn until the free slots run out, starting with the workspace
   that has gone longest without a call. Test with the counts in Acceptance.
10. `control_003.sql` adds `tenants.monthly_cap_micro_usd` (NULL = config default). `recapz workspace-cap <id>
    <usd>` sets it. `server_monthly_cap_usd` in the config sums `usage::month_totals` over all workspaces once per
    runner pass and pauses paid jobs everywhere when reached. `/api/usage` shows the workspace's effective cap.

## Open questions

- None blocking. Later, when a second customer team is real: deleting and exporting a whole workspace, joining by
  Google domain, and a notetaker account per workspace.
