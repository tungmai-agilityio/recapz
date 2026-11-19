import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { useSearchParams } from 'react-router'
import { disconnectCalendar, getCalendar, getJoinSettings, putAudioRetention, putJoinSettings, type JoinSettings } from '../api'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { inputClass } from '../ui/Field'
import { PageHead } from '../ui/PageHead'
import { useToast } from '../ui/toast-context'
import { useRecordingStorage, useSpeechProvider, useUsage } from '../speech'

/** Micro-USD (1,000,000 = $1) as dollars; sub-cent amounts keep four decimals. */
const usd = (micro: number) => `$${(micro / 1e6).toFixed(micro !== 0 && micro < 10_000 ? 4 : 2)}`

/** What the server redirects back with after Google's consent screen. */
const outcomes: Record<string, { tone: 'good' | 'bad'; text: string }> = {
  connected: { tone: 'good', text: 'Google Calendar is connected.' },
  denied: { tone: 'bad', text: 'Calendar access was not granted, so nothing was connected.' },
  scope_missing: { tone: 'bad', text: 'The calendar box was unticked on the Google screen, so nothing was connected.' },
  wrong_account: { tone: 'bad', text: 'That Google account is not the one you signed in with. Pick the same account and try again.' },
  error: { tone: 'bad', text: 'Google Calendar could not be connected. Try again.' },
}

const rules = [
  { id: 'manual', label: 'Manual', hint: 'The notetaker joins only meetings a member switched on.' },
  { id: 'auto', label: 'Automatic', hint: 'New meetings with a video link start switched on. A member can still switch one off.' },
] as const

/** Mirrors the server's notice: the provider name and the "opt out" wording are added when missing. */
function preview(message: string, name: string, provider: string) {
  let n = message.replaceAll('{name}', name).replaceAll('{provider}', provider)
  if (!n.includes(provider)) n += ` Audio is sent to ${provider}.`
  if (!n.toLowerCase().includes('opt out')) n += " Type 'opt out' to make it leave and delete the recording."
  return n
}

function JoinForm({ saved, admin, provider }: { saved: JoinSettings; admin: boolean; provider: string }) {
  const qc = useQueryClient()
  const toast = useToast()
  const [v, setV] = useState(saved)
  const save = useMutation({
    mutationFn: putJoinSettings,
    onSuccess: (s) => { toast('Join settings saved'); qc.setQueryData(['join-settings'], s) },
  })
  const err = save.isError ? save.error.message.replace(/^\d+ /, '') : ''
  const field = (re: RegExp) => (re.test(err) ? err : '')
  const known = /notetaker name|join message|join rule/
  const set = (k: keyof JoinSettings) => (e: { target: { value: string } }) => setV({ ...v, [k]: e.target.value } as JoinSettings)
  return (
    <div className="mt-4 rounded-lg border border-hair bg-sheet">
      <div className="grid gap-x-8 gap-y-3 p-5 @2xl:grid-cols-[minmax(0,264px)_1fr]">
        <div>
          <h2 className="font-semibold">Joining and the chat notice</h2>
          <p className="mt-1 text-sm text-mute">When the notetaker joins, what it is called, and what it tells the call.</p>
        </div>
        {!admin ? (
          <dl className="text-sm">
            <dt className="font-medium">Join rule</dt><dd>{rules.find((r) => r.id === saved.join_rule)?.label}: {rules.find((r) => r.id === saved.join_rule)?.hint}</dd>
            <dt className="mt-2 font-medium">Notetaker name</dt><dd>{saved.notetaker_name}</dd>
            <dt className="mt-2 font-medium">Join message</dt><dd>{preview(saved.join_message, saved.notetaker_name, provider)}</dd>
          </dl>
        ) : (
          <form className="text-sm" onSubmit={(e) => { e.preventDefault(); save.mutate(v) }}>
            <fieldset>
              <legend className="font-medium">Join rule</legend>
              {rules.map((r) => (
                <label key={r.id} className="mt-1 flex items-start gap-2">
                  <input type="radio" name="join-rule" className="mt-1" checked={v.join_rule === r.id} onChange={() => setV({ ...v, join_rule: r.id })} />
                  <span><b>{r.label}</b><span className="block text-mute">{r.hint}</span></span>
                </label>
              ))}
              {field(/join rule/) && <p role="alert" className="mt-1 text-bad">{err}</p>}
            </fieldset>
            <label htmlFor="notetaker-name" className="mt-4 block font-medium">Notetaker name</label>
            <input id="notetaker-name" className={`${inputClass} mt-1`} value={v.notetaker_name} onChange={set('notetaker_name')} />
            {field(/notetaker name/) && <p role="alert" className="mt-1 text-bad">{err}</p>}
            <label htmlFor="join-message" className="mt-4 block font-medium">Join message</label>
            <textarea id="join-message" rows={3} className={`${inputClass} mt-1`} value={v.join_message} onChange={set('join_message')} />
            <p className="mt-1 text-mute">{'{name}'} and {'{provider}'} are filled in. The provider name and the "opt out" wording are always kept.</p>
            {field(/join message/) && <p role="alert" className="mt-1 text-bad">{err}</p>}
            <p className="mt-2 font-medium">Preview</p>
            <p aria-label="Message preview" className="mt-1 rounded-md bg-hair-2 px-3 py-2">{preview(v.join_message, v.notetaker_name, provider)}</p>
            {err && !known.test(err) && <p role="alert" className="mt-2 text-bad">{err}</p>}
            <div className="mt-4"><Button type="submit" variant="primary" disabled={save.isPending}>Save</Button></div>
          </form>
        )}
      </div>
    </div>
  )
}

export function Recording() {
  const provider = useSpeechProvider()
  const storage = useRecordingStorage()
  const usage = useUsage().data
  const qc = useQueryClient()
  const toast = useToast()
  const [params, setParams] = useSearchParams()
  const outcome = outcomes[params.get('calendar') ?? '']
  const cal = useQuery({ queryKey: ['calendar'], queryFn: getCalendar })
  const keep = useMutation({
    mutationFn: putAudioRetention,
    onSuccess: () => { toast('Audio retention saved'); qc.invalidateQueries({ queryKey: ['audio-retention'] }) },
  })
  const join = useQuery({ queryKey: ['join-settings'], queryFn: getJoinSettings })
  const [asking, setAsking] = useState(false)
  const off = useMutation({
    mutationFn: disconnectCalendar,
    onSuccess: () => { setAsking(false); toast('Google Calendar disconnected'); qc.invalidateQueries({ queryKey: ['calendar'] }) },
  })
  // The outcome is read once from the address, then dropped so a reload does not repeat it.
  useEffect(() => { if (outcome) qc.invalidateQueries({ queryKey: ['calendar'] }) }, [outcome, qc])
  const dismiss = () => setParams((p) => { p.delete('calendar'); return p }, { replace: true })

  const c = cal.data
  const state = !c ? null : c.needs_reconnect ? 'reconnect' : c.connected ? 'connected' : 'none'
  return (
    <section>
      <PageHead title="Recording & privacy" lede="Who gets recorded, what people are told, and how long anything is kept." />
      {outcome && (
        <p role={outcome.tone === 'bad' ? 'alert' : undefined} aria-live={outcome.tone === 'bad' ? undefined : 'polite'}
          className={`mt-4 flex items-start justify-between gap-3 rounded-md px-3 py-2 text-sm ${outcome.tone === 'good' ? 'bg-good-wash text-good' : 'bg-bad-wash text-bad'}`}>
          <span>{outcome.text}</span>
          <button type="button" onClick={dismiss} className="font-medium underline">Dismiss</button>
        </p>
      )}
      <div className="mt-6 rounded-lg border border-hair bg-sheet">
        <div className="grid gap-x-8 gap-y-3 p-5 @2xl:grid-cols-[minmax(0,264px)_1fr]">
          <div>
            <h2 className="font-semibold">Calendar</h2>
            <p className="mt-1 text-sm text-mute">Recapz reads your Google Calendar to find meetings with a video link. It never changes your calendar.</p>
          </div>
          <div>
            <div className="flex items-center justify-between gap-3">
              <span aria-hidden className="flex h-7 w-7 items-center justify-center rounded-md bg-speaker-1 text-sm font-bold text-primary-fg">G</span>
              {state === 'connected' && <Badge tone="good">Connected</Badge>}
              {state === 'reconnect' && <Badge tone="bad">Reconnect needed</Badge>}
              {state === 'none' && <Badge>Not connected</Badge>}
            </div>
            <h3 className="mt-2 font-semibold">Google Calendar</h3>
            <p className="text-sm text-mute">
              {state === 'reconnect' ? 'Google stopped accepting the saved access, so new meetings are not being found.' : 'Finds meetings to join'}
            </p>
            {cal.isError && <p role="alert" className="mt-3 text-sm text-bad">Could not load the calendar connection.</p>}
            {state && (
              <div className="mt-4 flex flex-wrap gap-2">
                {state !== 'connected' && (
                  <a href="/auth/calendar" className="inline-flex items-center rounded-md border border-primary bg-primary px-3 py-1.5 text-sm font-medium text-primary-fg no-underline hover:opacity-90">
                    {state === 'reconnect' ? 'Reconnect Google Calendar' : 'Connect Google Calendar'}
                  </a>
                )}
                {state !== 'none' && <Button variant="danger" onClick={() => setAsking(true)}>Disconnect</Button>}
              </div>
            )}
          </div>
        </div>
      </div>
      {join.isError && <p role="alert" className="mt-4 text-sm text-bad">Could not load the join settings.</p>}
      {join.isPending && <p className="mt-4 text-sm text-mute">Loading join settings...</p>}
      {join.data?.join_rule && <JoinForm key={JSON.stringify(join.data)} saved={join.data} admin={storage.isAdmin} provider={provider} />}
      <div className="mt-4 rounded-lg border border-hair bg-sheet">
        <div className="grid gap-x-8 gap-y-3 p-5 @2xl:grid-cols-[minmax(0,264px)_1fr]">
          <div>
            <h2 className="font-semibold">Where audio goes</h2>
            <p className="mt-1 text-sm text-mute">Call and upload audio leaves this server only for transcription.</p>
          </div>
          <p className="text-sm">Audio is sent to <b>{provider}</b> for transcription, live during a call and again from the recorded file afterwards. Nothing else is sent to it.</p>
          {storage.where && (
            <p className="text-sm @2xl:col-start-2">
              Finished recordings are kept {storage.where === 'object_storage' ? 'in private object storage' : 'on this server\'s disk'}
              {storage.days === undefined ? '.' : storage.days === 0 ? ' and their audio is kept until someone deletes the meeting.' : <> and their audio is deleted after <b>{storage.days} days</b>.</>}
              {' '}Transcripts and reports stay.
            </p>
          )}
          {storage.isAdmin && storage.days !== undefined && (
            <div className="text-sm @2xl:col-start-2">
              <label htmlFor="audio-retention" className="block font-medium">Delete audio after</label>
              <select id="audio-retention" className={`${inputClass} mt-1`} value={storage.days} disabled={keep.isPending}
                onChange={(e) => keep.mutate(Number(e.target.value))}>
                {[...new Set([30, 90, 180, 365, storage.days])].sort((a, b) => a - b).filter((d) => d > 0).map((d) => <option key={d} value={d}>{d} days</option>)}
                <option value={0}>Keep until the meeting is deleted</option>
              </select>
              <p className="mt-1 text-mute">Audio older than this is deleted at the next hourly sweep. Transcripts and reports stay.</p>
              {keep.isError && <p role="alert" className="mt-1 text-bad">{keep.error.message.replace(/^\d+ /, '')}</p>}
            </div>
          )}
        </div>
      </div>
      {usage && (
        <div className="mt-4 rounded-lg border border-hair bg-sheet">
          <div className="grid gap-x-8 gap-y-3 p-5 @2xl:grid-cols-[minmax(0,264px)_1fr]">
            <div>
              <h2 className="font-semibold">Usage this month</h2>
              <p className="mt-1 text-sm text-mute">Estimated from list prices, not an invoice. Only admins see this.</p>
            </div>
            <div className="text-sm">
              {usage.paused && (
                <p role="alert" className="mb-3 rounded-md bg-bad-wash px-3 py-2 text-bad">
                  The monthly cap is reached. New transcripts, reports and search indexing wait until next month or a higher cap.
                </p>
              )}
              <p>
                <b>{usd(usage.total_micro_usd)}</b>
                {usage.cap_micro_usd > 0 ? <> of {usd(usage.cap_micro_usd)} cap</> : ' (no cap set)'}
                {' '}· speech {usd(usage.speech_micro_usd)} · language model {usd(usage.llm_micro_usd)}
              </p>
              {usage.meetings.length > 0 && (
                <ul className="mt-3 divide-y divide-hair" aria-label="Cost per meeting">
                  {usage.meetings.map((m) => (
                    <li key={m.id} className="flex justify-between gap-3 py-1.5">
                      <span className="truncate">{m.title}</span>
                      <span className="font-mono">{usd(m.speech_micro_usd + m.llm_micro_usd)}</span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>
      )}
      {asking && (
        <ConfirmDialog title="Disconnect Google Calendar?" confirmLabel="Disconnect" busy={off.isPending}
          error={off.isError ? 'Could not disconnect. Try again.' : undefined}
          onConfirm={() => off.mutate()} onCancel={() => { setAsking(false); off.reset() }}>
          Recapz forgets the saved access and stops finding your meetings. Meetings already recorded stay.
        </ConfirmDialog>
      )}
    </section>
  )
}
