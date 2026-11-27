import { expect, test } from '@playwright/test'

/** Silence as 16 kHz mono 16-bit PCM; the stub speech provider ignores the audio. */
function silentWav(seconds: number) {
  const data = 16000 * 2 * seconds
  const b = Buffer.alloc(44 + data)
  b.write('RIFF', 0)
  b.writeUInt32LE(36 + data, 4)
  b.write('WAVEfmt ', 8)
  b.writeUInt32LE(16, 16)
  b.writeUInt16LE(1, 20)
  b.writeUInt16LE(1, 22)
  b.writeUInt32LE(16000, 24)
  b.writeUInt32LE(32000, 28)
  b.writeUInt16LE(2, 32)
  b.writeUInt16LE(16, 34)
  b.write('data', 36)
  b.writeUInt32LE(data, 40)
  return b
}

test('sign in, upload to ready, seek from the transcript, Ask opens the cited moment', async ({ page }) => {
  // The sign-in form exists only in the Vite dev server, so sign in through the same endpoint it calls.
  await page.goto('/')
  await expect(page.getByRole('heading', { name: 'Sign in to Recapz' })).toBeVisible()
  const login = await page.request.post('/auth/dev-login', { data: { email: 'e2e@example.com' } })
  expect(login.status()).toBe(200)
  await page.reload()
  await expect(page.getByRole('heading', { name: 'Sign in to Recapz' })).toHaveCount(0)

  await page.goto('/upload')
  await page.locator('#file').setInputFiles({ name: 'launch.wav', mimeType: 'audio/wav', buffer: silentWav(7) })
  await page.getByLabel('Title').fill('Launch sync')
  await page.getByLabel('Language').selectOption('en')
  await page.getByRole('button', { name: 'Process recording' }).click()
  await page.getByRole('link', { name: 'Open the meeting' }).click({ timeout: 60_000 })

  await expect(page.getByText('The team plans the March launch.')).toBeVisible({ timeout: 30_000 })
  await page.getByRole('tab', { name: 'Transcript' }).click()
  const second = page.getByRole('button', { name: /Sam owns the payment testing/ })
  await second.click()
  await expect(second).toHaveAttribute('aria-current', 'true')
  await expect(page.getByRole('button', { name: /Hi Sam, we launch/ })).not.toHaveAttribute('aria-current')

  await page.goto('/ask')
  await page.getByLabel('Question').fill('When do we launch?')
  await page.getByRole('button', { name: 'Ask', exact: true }).click()
  await expect(page.getByText(/We launch on March fourteenth/)).toBeVisible({ timeout: 30_000 })
  await page.locator('#cite-0-0').click()
  await expect(page).toHaveURL(/\/meetings\/[^?]+\?t=\d+/)
  await expect(page.getByRole('heading', { name: 'Launch sync' })).toBeVisible()
})
