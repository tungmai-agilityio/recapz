import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { DocNotFound, deleteDocument, documentFileUrl, getDocument, setDocumentLlmOk } from '../api'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { buttonClass } from '../ui/buttonClass'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { PageHead } from '../ui/PageHead'
import { Switch } from '../ui/Switch'
import { addedOn, bytesLabel, label, tone } from './documentFormat'

/** One uploaded document: its details, the text as plain wrapped lines, and the original to download. */
export function Document() {
  const { id = '' } = useParams()
  const qc = useQueryClient()
  const nav = useNavigate()
  const [confirm, setConfirm] = useState(false)
  const doc = useQuery({
    queryKey: ['documents', 'detail', id],
    queryFn: () => getDocument(id),
    retry: (n, e) => !(e instanceof DocNotFound) && n < 2,
    refetchInterval: (q) => (q.state.data?.status === 'indexing' ? 2000 : false),
  })
  const refresh = () => qc.invalidateQueries({ queryKey: ['documents'] })
  const toggle = useMutation({ mutationFn: (on: boolean) => setDocumentLlmOk(id, on), onSettled: refresh })
  const remove = useMutation({ mutationFn: () => deleteDocument(id), onSuccess: async () => { await refresh(); nav('/documents') } })

  const back = <Link to="/documents" className="text-sm text-mute hover:text-primary">← My documents</Link>
  if (doc.isPending) return <p className="text-mute">Loading…</p>
  if (doc.error instanceof DocNotFound) return <section className="flex flex-col gap-3">{back}<p className="text-mute">Document not found.</p></section>
  if (!doc.data) return <section className="flex flex-col gap-3">{back}<p role="alert" className="text-sm text-bad">Could not load this document.</p></section>
  const d = doc.data

  return (
    <section className="flex flex-col gap-6">
      <div className="flex flex-col gap-1.5">
        {back}
        <PageHead title={d.name} actions={<>
          <a href={documentFileUrl(d.id)} download className={buttonClass()}>Download</a>
          <Button variant="danger" onClick={() => { remove.reset(); setConfirm(true) }}>Delete</Button>
        </>} />
      </div>
      <dl className="frame flex flex-wrap items-center gap-x-8 gap-y-3 text-sm">
        {([['Type', d.kind === 'md' ? 'Markdown' : 'TXT'], ['Size', bytesLabel(d.size_bytes)], ['Added', addedOn(d.created_at)]] as const).map(([k, v]) => (
          <div key={k}><dt className="text-xs text-mute">{k}</dt><dd className="font-medium">{v}</dd></div>
        ))}
        <div><dt className="text-xs text-mute">Status</dt><dd><Badge tone={tone[d.status]}>{label[d.status]}</Badge></dd></div>
        {d.status !== 'no text found' && (
          <div className="flex items-center gap-2">
            <Switch label="May be sent to the language model" checked={d.llm_ok} onChange={(on) => toggle.mutate(on)} />
            <span>May be sent to the language model</span>
          </div>
        )}
      </dl>
      {toggle.isError && <p role="alert" className="text-sm text-bad">Could not change that setting.</p>}
      <div className="frame flex flex-col gap-3">
        {d.text ? <pre className="font-sans text-[.95rem] leading-7 break-words whitespace-pre-wrap">{d.text}</pre> : <p className="text-sm text-mute">This document has no text.</p>}
        {d.truncated && (
          <p className="border-t border-hair pt-3 text-sm text-mute">
            Only the first 1 MB is shown. <a href={documentFileUrl(d.id)} download className="text-primary underline">Download</a> the whole file.
          </p>
        )}
      </div>
      {confirm && (
        <ConfirmDialog title="Delete this document?" confirmLabel="Delete" busy={remove.isPending}
          error={remove.isError ? 'Could not delete. Try again.' : undefined}
          onConfirm={() => remove.mutate()} onCancel={() => setConfirm(false)}>
          {d.name} and its search index are deleted for good.
        </ConfirmDialog>
      )}
    </section>
  )
}
