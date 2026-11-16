import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, test, vi } from 'vitest'
import { Badge } from './Badge'
import { Faces, Face } from './Face'
import { Menu, MenuItem } from './Menu'
import { PageHead } from './PageHead'
import { ToastProvider } from './Toast'
import { useToast } from './toast-context'

afterEach(() => { cleanup(); vi.useRealTimers() })

test('Badge tones map to their token classes', () => {
  render(<><Badge tone="good">Ready</Badge><Badge tone="bad">Failed</Badge></>)
  expect(screen.getByText('Ready').className).toContain('text-good')
  expect(screen.getByText('Failed').className).toContain('text-bad')
})

test('Face shows the uppercase initial with the speaker colour, cycling after five', () => {
  render(<><Face name="linh" speaker={1} /><Face name="Minh" speaker={5} /></>)
  const a = screen.getByRole('img', { name: 'linh' })
  expect(a.textContent).toBe('L')
  expect(a.className).toContain('bg-speaker-2')
  expect(screen.getByRole('img', { name: 'Minh' }).className).toContain('bg-speaker-1')
})

test('Faces caps the stack and counts the rest', () => {
  render(<Faces people={['A', 'B', 'C', 'D', 'E', 'F']} max={4} />)
  expect(screen.getAllByRole('img')).toHaveLength(4)
  expect(screen.getByText('+2')).toBeTruthy()
})

test('PageHead renders heading, lede and actions', () => {
  render(<PageHead title="Meetings" lede="38 meetings this month" actions={<button>Upload</button>} />)
  expect(screen.getByRole('heading', { level: 1 }).textContent).toBe('Meetings')
  expect(screen.getByText('38 meetings this month')).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Upload' })).toBeTruthy()
})

test('Menu opens, closes on Esc, outside click and item pick', () => {
  const pick = vi.fn()
  render(<><Menu label="Account" trigger="T"><MenuItem onClick={pick}>Sign out</MenuItem></Menu><p>outside</p></>)
  const trigger = screen.getByRole('button', { name: 'Account' })
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.click(trigger)
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  fireEvent.keyDown(document, { key: 'Escape' })
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.click(trigger)
  fireEvent.mouseDown(screen.getByText('outside'))
  expect(screen.queryByRole('menu')).toBeNull()
  fireEvent.click(trigger)
  fireEvent.click(screen.getByRole('menuitem', { name: 'Sign out' }))
  expect(pick).toHaveBeenCalledTimes(1)
  expect(screen.queryByRole('menu')).toBeNull()
})

function Fire() {
  const toast = useToast()
  return <button onClick={() => toast('Saved')}>go</button>
}

test('Toast announces the message and clears it after 3 seconds', () => {
  vi.useFakeTimers()
  render(<ToastProvider><Fire /></ToastProvider>)
  fireEvent.click(screen.getByText('go'))
  expect(screen.getByRole('status').textContent).toBe('Saved')
  act(() => { vi.advanceTimersByTime(3000) })
  expect(screen.getByRole('status').textContent).toBe('')
})

test('Toast firing the same message again restarts the 3 second window', () => {
  vi.useFakeTimers()
  render(<ToastProvider><Fire /></ToastProvider>)
  fireEvent.click(screen.getByText('go'))
  act(() => { vi.advanceTimersByTime(2000) })
  fireEvent.click(screen.getByText('go'))
  act(() => { vi.advanceTimersByTime(2000) })
  expect(screen.getByRole('status').textContent).toBe('Saved')
  act(() => { vi.advanceTimersByTime(1000) })
  expect(screen.getByRole('status').textContent).toBe('')
})

test('Menu side="below" opens downward, the default stays above', () => {
  render(<><Menu label="Down" side="below" trigger="D"><MenuItem>x</MenuItem></Menu><Menu label="Up" trigger="U"><MenuItem>y</MenuItem></Menu></>)
  fireEvent.click(screen.getByRole('button', { name: 'Down' }))
  fireEvent.click(screen.getByRole('button', { name: 'Up' }))
  expect(screen.getByRole('menu', { name: 'Down' }).className).toContain('top-full')
  expect(screen.getByRole('menu', { name: 'Up' }).className).toContain('bottom-full')
})

test('Menu align="left" anchors the panel to the trigger start, the default to its end', () => {
  render(<><Menu label="L" align="left" trigger="L"><MenuItem>x</MenuItem></Menu><Menu label="R" trigger="R"><MenuItem>y</MenuItem></Menu></>)
  fireEvent.click(screen.getByRole('button', { name: 'L' }))
  fireEvent.click(screen.getByRole('button', { name: 'R' }))
  expect(screen.getByRole('menu', { name: 'L' }).className).toContain('left-0')
  expect(screen.getByRole('menu', { name: 'R' }).className).toContain('right-0')
})
