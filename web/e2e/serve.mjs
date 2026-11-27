// Starts the stub speech/LLM providers and the real recapz server pointed at them. Playwright's webServer runs this.
import { spawn } from 'node:child_process'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const STUB_PORT = 18081
const root = resolve(import.meta.dirname, '../..')

const UTTERANCES = [
  { start: 0, end: 3, speaker: 0, transcript: 'Hi Sam, we launch on March fourteenth.' },
  { start: 3, end: 6, speaker: 1, transcript: 'Sam owns the payment testing.' },
]

/** Reply of the language model by task, told apart by the first words of the system prompt. */
function completion(system) {
  const replies = [
    ['You answer one question', { answer: 'We launch on March fourteenth [1].', citations: [1] }],
    ['You write the summary', { summary: 'The team plans the March launch.' }],
    ['You extract decisions', { decisions: ['Launch on March fourteenth'], open_questions: [] }],
    ['You split one meeting', { chapters: [{ start_ms: 0, title: 'Launch plan' }] }],
    ['You extract action items', { action_items: [{ text: 'Test payments', owner: 'Sam', due: null, source_ms: 3000 }] }],
  ]
  const hit = replies.find(([start]) => system.startsWith(start))
  return JSON.stringify(hit ? hit[1] : {})
}

const stub = createServer((req, res) => {
  const chunks = []
  req.on('data', (c) => chunks.push(c))
  req.on('end', () => {
    res.setHeader('content-type', 'application/json')
    if (req.url.startsWith('/v1/listen')) return res.end(JSON.stringify({ results: { utterances: UTTERANCES } }))
    if (req.url === '/v1/chat/completions') {
      const { messages } = JSON.parse(Buffer.concat(chunks).toString())
      const content = completion(messages[0].content)
      return res.end(JSON.stringify({ choices: [{ message: { content } }], usage: { prompt_tokens: 1, completion_tokens: 1 } }))
    }
    res.statusCode = 404
    res.end('{}')
  })
})
stub.listen(STUB_PORT, '127.0.0.1')

const dir = mkdtempSync(join(tmpdir(), 'recapz-e2e-'))
const stubUrl = `http://127.0.0.1:${STUB_PORT}`
writeFileSync(join(dir, 'config.toml'), `data_dir = "${dir}/data"
base_url = "http://127.0.0.1:18080"
bind = "127.0.0.1:18080"
bootstrap_admin_email = "e2e@example.com"
dev_login = true
[speech]
base_url = "${stubUrl}"
[llm]
provider = "openai"
base_url = "${stubUrl}/v1"
model = "stub"
`)

const server = spawn('cargo', ['run', '-q', '-p', 'recapz-server'], {
  cwd: root,
  stdio: 'inherit',
  env: { ...process.env, RECAPZ_CONFIG: join(dir, 'config.toml'), RECAPZ_SPEECH_KEY: 'stub', RECAPZ_LLM_KEY: 'stub' },
})
const stop = () => { server.kill(); stub.close(); process.exit(0) }
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
server.on('exit', (code) => process.exit(code ?? 1))
