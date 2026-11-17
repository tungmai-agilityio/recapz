import { useMutation, useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { Link } from 'react-router'
import { getMeeting, uploadRecording } from '../api'
import { Button } from '../ui/Button'
import { Field, inputClass } from '../ui/Field'
import { PageHead } from '../ui/PageHead'

/** Formats the server's decoder reads, and its upload limit (crates/speech, crates/server). */
const EXTENSIONS = ['mp3', 'm4a', 'aac', 'wav', 'mp4', 'ogg', 'flac', 'mkv']
const MAX_BYTES = 2 * 1024 * 1024 * 1024

const STEPS = [['uploaded', 'Uploaded'], ['transcribing', 'Transcribing'], ['reporting', 'Report'], ['ready', 'Ready']] as const

function problem(f: File): string | null {
  const ext = f.name.split('.').pop()?.toLowerCase() ?? ''
  if (!EXTENSIONS.includes(ext)) return `Unsupported format. Use ${EXTENSIONS.join(', ').toUpperCase()}.`
  if (f.size > MAX_BYTES) return 'This file is over 2 GB.'
  return null
}

export function Upload() {
  const [file, setFile] = useState<File | null>(null)
  const [title, setTitle] = useState('')
  const [over, setOver] = useState(false)
  const bad = file ? problem(file) : null
  const send = useMutation({ mutationFn: () => uploadRecording(file!, title) })
  const id = send.data?.id
  const { data: m } = useQuery({
    queryKey: ['upload-status', id],
    queryFn: () => getMeeting(id!),
    enabled: !!id,
    refetchInterval: (q) => (['ready', 'failed'].includes(q.state.data?.status ?? '') ? false : 2000),
  })
  const status = m?.status ?? 'uploaded'
  const at = STEPS.findIndex(([s]) => s === status)

  return (
    <section className="flex flex-col gap-7">
      <PageHead title="Upload a recording"
        lede="For in-person meetings or calls the notetaker didn't join. Processed on your server." />
      <div className="grid grid-cols-[minmax(0,1fr)_320px] items-start gap-9 max-[900px]:grid-cols-1 max-[900px]:gap-7">
        <div className="flex flex-col gap-5">
          <label htmlFor="file" onDragOver={(e) => { e.preventDefault(); setOver(true) }} onDragLeave={() => setOver(false)}
            onDrop={(e) => { e.preventDefault(); setOver(false); setFile(e.dataTransfer.files[0] ?? null); send.reset() }}
            className={`flex cursor-pointer flex-col items-center gap-2 rounded-[10px] border-[1.5px] border-dashed bg-sheet px-5 py-9 text-center hover:border-ink-2 ${over ? 'border-ink-2' : 'border-hair'}`}>
            <b>{file ? file.name : 'Drop a file or choose one'}</b>
            <span className="text-[.82rem] text-mute">
              {EXTENSIONS.map((e) => e.toUpperCase()).join(' · ')} · up to 2 GB
            </span>
            <input id="file" type="file" className="max-w-full text-[.8rem]" accept={EXTENSIONS.map((e) => `.${e}`).join(',')}
              onChange={(e) => { setFile(e.target.files?.[0] ?? null); send.reset() }} />
          </label>
          {bad && <p role="alert" className="text-sm text-bad">{bad}</p>}
          {(send.isPending || id) && (
            <ol aria-label="Progress" className="grid grid-cols-4 gap-1.5 max-[900px]:grid-cols-1">
              {STEPS.map(([s, label], i) => {
                const state = status === 'failed' ? (i < 1 ? 'done' : '') : status === 'ready' || i < at ? 'done' : i === at ? 'run' : ''
                return (
                  <li key={s} data-state={state} className={`flex flex-col gap-1.5 text-[.8rem] ${state ? 'text-ink' : 'text-mute'}`}>
                    <span className={`h-1 rounded-sm ${state === 'done' ? 'bg-ink' : state === 'run' ? 'animate-pulse bg-ink-2' : 'bg-hair'}`} />
                    {label}
                  </li>
                )
              })}
            </ol>
          )}
          {status === 'failed' && (
            <p role="alert" className="text-sm text-bad">Processing failed{m?.error ? `: ${m.error}` : '.'} Press Process recording to try again.</p>
          )}
          {send.isError && <p role="alert" className="text-sm text-bad">Upload failed. Try again.</p>}
          {id && status === 'ready' && <Link to={`/meetings/${id}`} className="text-sm font-medium">Open the meeting</Link>}
        </div>
        <aside className="flex flex-col gap-4">
          <Field label="Title" hint="Defaults to the file name.">
            {(p) => <input {...p} className={inputClass} value={title} onChange={(e) => setTitle(e.target.value)} />}
          </Field>
          <Button variant="primary" className="justify-center" disabled={!file || !!bad || send.isPending} onClick={() => { send.reset(); send.mutate() }}>
            {send.isPending ? 'Uploading…' : 'Process recording'}
          </Button>
        </aside>
      </div>
    </section>
  )
}
