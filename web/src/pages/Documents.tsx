import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { deleteDocument, getDocuments, setDocumentLlmOk, uploadDocument, type Doc } from '../api'
import { Badge } from '../ui/Badge'
import { Button } from '../ui/Button'
import { ConfirmDialog } from '../ui/ConfirmDialog'
import { PageHead } from '../ui/PageHead'
import { Switch } from '../ui/Switch'
import { Cell, Row, Table } from '../ui/Table'
import { useSpeechProvider } from '../speech'

/** What the server accepts (crates/core document.rs): Markdown and TXT, 20 MB each. */
const EXTENSIONS = ['md', 'markdown', 'txt']
const MAX_BYTES = 20 * 1024 * 1024

function problem(f: File): string | null {
  const ext = f.name.split('.').pop()?.toLowerCase() ?? ''
  if (!EXTENSIONS.includes(ext)) return `${f.name}: unsupported format. Use Markdown or TXT.`
  if (f.size > MAX_BYTES) return `${f.name}: over 20 MB.`
  return null
}

const tone = { indexing: 'signal', indexed: 'good', 'no text found': 'neutral' } as const
const label = { indexing: 'Indexing', indexed: 'Indexed', 'no text found': 'No text found' } as const

export function Documents() {
  const qc = useQueryClient()
  const provider = useSpeechProvider()
  const [over, setOver] = useState(false)
  const [errors, setErrors] = useState<string[]>([])
  const [del, setDel] = useState<Doc | null>(null)
  const docs = useQuery({
    queryKey: ['documents'],
    queryFn: getDocuments,
    refetchInterval: (q) => (q.state.data?.some((d) => d.status === 'indexing') ? 2000 : false),
  })
  const refresh = () => qc.invalidateQueries({ queryKey: ['documents'] })
  const add = useMutation({
    mutationFn: async (files: File[]) => {
      const bad: string[] = []
      for (const f of files) {
        const p = problem(f)
        if (p) { bad.push(p); continue }
        try { await uploadDocument(f) } catch { bad.push(`${f.name}: upload failed.`) }
      }
      setErrors(bad)
      await refresh()
    },
  })
  const toggle = useMutation({ mutationFn: (v: { id: string; on: boolean }) => setDocumentLlmOk(v.id, v.on), onSettled: refresh })
  const remove = useMutation({ mutationFn: (id: string) => deleteDocument(id), onSuccess: () => { setDel(null); return refresh() } })
  const pick = (list: FileList | null) => { if (list?.length) add.mutate([...list]) }

  return (
    <section className="flex flex-col gap-7">
      <PageHead title="My documents" lede="Notes you add here can be searched together with your meetings in Ask. Stored on your server." />
      <div className="grid grid-cols-[minmax(0,1fr)_320px] items-start gap-9 max-[900px]:grid-cols-1 max-[900px]:gap-7">
        <div className="frame flex flex-col gap-5">
          <label htmlFor="docs" onDragOver={(e) => { e.preventDefault(); setOver(true) }} onDragLeave={() => setOver(false)}
            onDrop={(e) => { e.preventDefault(); setOver(false); pick(e.dataTransfer.files) }}
            className={`flex cursor-pointer flex-col items-center gap-2 rounded-[10px] border-[1.5px] border-dashed bg-inset px-5 py-9 text-center hover:border-ink-2 ${over ? 'border-ink-2' : 'border-hair'}`}>
            <b>{add.isPending ? 'Uploading…' : 'Drop files or choose some'}</b>
            <span className="text-[.82rem] text-mute">MD · TXT · up to 20 MB each</span>
            <input id="docs" type="file" multiple className="max-w-full text-[.8rem]" accept={EXTENSIONS.map((e) => `.${e}`).join(',')}
              onChange={(e) => { pick(e.target.files); e.target.value = '' }} />
          </label>
          {errors.map((m) => <p key={m} role="alert" className="text-sm text-bad">{m}</p>)}
          {docs.isError && <p role="alert" className="text-sm text-bad">Could not load your documents.</p>}
          {docs.data?.length === 0 && <p className="text-sm text-mute">No documents yet.</p>}
          {!!docs.data?.length && (
            <Table label="Documents" head={['Name', 'Type', 'Status', 'Send to language model', '']}>
              {docs.data.map((d) => (
                <Row key={d.id}>
                  <Cell className="font-medium break-all">{d.name}</Cell>
                  <Cell>{(d.name.split('.').pop() ?? '').toUpperCase()}</Cell>
                  <Cell><Badge tone={tone[d.status]}>{label[d.status]}</Badge></Cell>
                  <Cell>
                    {d.status === 'no text found'
                      ? <span className="text-mute">n/a</span>
                      : <Switch label={`May be sent to the language model: ${d.name}`} checked={d.llm_ok} onChange={(on) => toggle.mutate({ id: d.id, on })} />}
                  </Cell>
                  <Cell><Button variant="danger" onClick={() => { remove.reset(); setDel(d) }}>Delete</Button></Cell>
                </Row>
              ))}
            </Table>
          )}
          {toggle.isError && <p role="alert" className="text-sm text-bad">Could not change that setting.</p>}
        </div>
        <aside className="frame flex flex-col gap-3 text-sm text-ink-2">
          <b className="text-ink">May be sent to the language model</b>
          <p>Off (the default): the document is only searched on your server and is never sent anywhere.</p>
          <p>On: the passages that match a question you ask can be sent to the language model provider to write the answer. Audio goes only to {provider}, never to the language model.</p>
        </aside>
      </div>
      {del && (
        <ConfirmDialog title="Delete this document?" confirmLabel="Delete" busy={remove.isPending}
          error={remove.isError ? 'Could not delete. Try again.' : undefined}
          onConfirm={() => remove.mutate(del.id)} onCancel={() => setDel(null)}>
          {del.name} and its search index are deleted for good.
        </ConfirmDialog>
      )}
    </section>
  )
}
