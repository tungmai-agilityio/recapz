import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState, type FormEvent } from 'react'
import { cancelInvite, getMe, getMembers, inviteMember, removeMember, setMemberRole, Unauthorized, type Role } from '../api'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { Face } from '../ui/Face'
import { Field, inputClass } from '../ui/Field'
import { PageHead } from '../ui/PageHead'
import { Cell, Row, Table } from '../ui/Table'
import { useToast } from '../ui/toast-context'

/** The server's refusal text without its leading status code ("409 a workspace needs ..."). */
/** The server's refusal text without its leading status code ("409 a workspace needs ..."). */
/** Pulls the select's box left so its text lines up with the column header (header and cells pad 10px). */
const inCell = `${inputClass} @2xl:-ml-[11px]`
const reason = (e: unknown, fallback: string) => (e instanceof Error ? e.message.replace(/^\d+\s+/, '') : '') || fallback



export function Members() {
  const qc = useQueryClient()
  const toast = useToast()
  const me = useQuery({ queryKey: ['me'], queryFn: getMe })
  const list = useQuery({ queryKey: ['members'], queryFn: getMembers, enabled: me.data?.role === 'admin' })
  const refresh = () => qc.invalidateQueries({ queryKey: ['members'] })
  const [email, setEmail] = useState('')
  const [role, setRole] = useState<Role>('member')
  const [link, setLink] = useState('')
  // An id or an invite email: the row whose Remove or Cancel invite waits for its inline confirm.
  const [confirm, setConfirm] = useState<string | null>(null)
  const [error, setError] = useState('')

  const invite = useMutation({
    mutationFn: () => inviteMember(email.trim(), role),
    onSuccess: (r) => { setLink(r.link); setEmail(''); setError(''); return refresh() },
    onError: (e) => setError(reason(e, 'Could not invite.')),
  })
  const change = useMutation({
    mutationFn: (v: { id: string; role: Role }) => setMemberRole(v.id, v.role),
    onSuccess: () => toast('Role changed'),
    onError: (e) => toast(reason(e, 'Could not change the role.')),
    onSettled: refresh,
  })
  const drop = useMutation({
    mutationFn: (key: string) => (key.includes('@') ? cancelInvite(key) : removeMember(key)),
    onSuccess: () => { setConfirm(null); return refresh() },
    onError: (e) => { setConfirm(null); toast(reason(e, 'Could not remove.')) },
  })
  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (!email.includes('@')) return setError('Enter a valid email.')
    invite.mutate()
  }
  const copy = async () => {
    try { await navigator.clipboard.writeText(link); toast('Link copied') } catch { toast('Could not copy. Select the link and copy it.') }
  }

  const ask = (key: string, label: string, aria: string) => confirm === key
    ? (
      <span className="flex flex-wrap items-center gap-2">
        <Button variant="danger" disabled={drop.isPending} onClick={() => drop.mutate(key)} aria-label={`Confirm: ${aria}`}>Yes, {label.toLowerCase()}</Button>
        <Button onClick={() => setConfirm(null)}>Keep</Button>
      </span>
    )
    : <Button variant="danger" onClick={() => setConfirm(key)} aria-label={aria}>{label}</Button>

  if (me.data && me.data.role !== 'admin')
    return <section className="flex flex-col gap-7"><PageHead title="Members" /><p className="text-mute">Only an admin can manage members.</p></section>

  return (
    <section className="flex flex-col gap-7">
      <PageHead title="Members" lede="Invite people by email, then send them the link: they sign in with Google as that email to join." />
      <form onSubmit={submit} noValidate className="frame flex flex-wrap items-start gap-3" aria-label="Invite">
        <div className="min-w-56 flex-1 max-sm:basis-full">
          <Field label="Email" error={error}>
            {(p) => <input {...p} type="email" className={`${inputClass} w-full`} value={email} onChange={(e) => setEmail(e.target.value)} />}
          </Field>
        </div>
        <Field label="Role">
          {(p) => (
            <select {...p} className={inputClass} value={role} onChange={(e) => setRole(e.target.value as Role)}>
              <option value="member">Member</option><option value="admin">Admin</option>
            </select>
          )}
        </Field>
        <div className="flex flex-col gap-1.5">
          <span aria-hidden className="text-sm select-none">&nbsp;</span>
          <Button type="submit" variant="primary" disabled={invite.isPending}>Invite</Button>
        </div>
        {link && (
          <p className="flex w-full flex-wrap items-center gap-2 text-sm text-ink-2">
            <span className="min-w-0 break-all">Invite sent. Share this link: <b className="text-ink">{link}</b></span>
            <Button onClick={copy}>Copy link</Button>
          </p>
        )}
      </form>
      {list.isError && !(list.error instanceof Unauthorized) && <p role="alert" className="text-sm text-bad">Could not load the members.</p>}
      {list.data && (
        <div className="frame">
          <Table label="Members" head={['Member', 'Email', 'Role', 'Meetings', '']}>
            {list.data.members.map((m) => {
              const you = m.email === me.data?.email
              const name = m.name || m.email
              return (
                <Row key={m.id}>
                  <Cell label="Member" className="font-semibold @max-2xl:col-span-2"><span className="flex items-center gap-2.5"><Face name={name} speaker={null} size={28} />{name}{you && <Badge>You</Badge>}</span></Cell>
                  <Cell label="Email" className="break-all text-ink-2 @max-2xl:col-span-2">{m.email}</Cell>
                  <Cell label="Role">
                    <select aria-label={`Role of ${name}`} className={inCell} value={m.role} disabled={change.isPending}
                      onChange={(e) => change.mutate({ id: m.id, role: e.target.value as Role })}>
                      <option value="member">Member</option><option value="admin">Admin</option>
                    </select>
                  </Cell>
                  <Cell label="Meetings">{m.meetings}</Cell>
                  <Cell className="@max-2xl:col-span-2">{!you && ask(m.id, 'Remove', `Remove ${name}`)}</Cell>
                </Row>
              )
            })}
            {list.data.invites.map((i) => (
              <Row key={i.email}>
                <Cell label="Member" className="font-semibold @max-2xl:col-span-2"><span className="flex items-center gap-2.5"><Face name={i.email} speaker={null} size={28} />{i.email}<Badge tone="signal">Pending</Badge></span></Cell>
                <Cell label="Email" className="break-all text-ink-2 @max-2xl:col-span-2">{i.email}</Cell>
                <Cell label="Role">
                  <select aria-label={`Role of ${i.email}`} className={inCell} value={i.role} disabled>
                    <option value="member">Member</option><option value="admin">Admin</option>
                  </select>
                </Cell>
                <Cell label="Meetings"><span className="text-mute">0</span></Cell>
                <Cell className="@max-2xl:col-span-2">{ask(i.email, 'Cancel invite', `Cancel invite for ${i.email}`)}</Cell>
              </Row>
            ))}
          </Table>
          <p className="mt-3 text-xs text-mute">Removing a member signs them out and moves their meetings to you.</p>
        </div>
      )}
    </section>
  )
}
