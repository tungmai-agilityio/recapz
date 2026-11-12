import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { afterEach, expect, test, vi } from 'vitest'
import { Button } from './Button'
import { ConfirmDialog } from './ConfirmDialog'
import { Choice } from './Choice'
import { Field } from './Field'
import { Switch } from './Switch'
import { Tabs } from './Tabs'

afterEach(cleanup)

test('Button defaults to type=button and primary uses the primary tokens', () => {
  render(<><Button>Plain</Button><Button variant="primary">Go</Button></>)
  expect(screen.getByRole('button', { name: 'Plain' }).getAttribute('type')).toBe('button')
  expect(screen.getByRole('button', { name: 'Go' }).className).toContain('bg-primary')
})

test('Field links label, hint and error to the control', () => {
  const { rerender } = render(
    <Field label="Title" hint="Shown in the list">{(p) => <input {...p} />}</Field>,
  )
  const input = screen.getByLabelText('Title')
  expect(input.getAttribute('aria-describedby')).toBe(screen.getByText('Shown in the list').id)
  expect(input.getAttribute('aria-invalid')).toBeNull()
  rerender(<Field label="Title" hint="Shown in the list" error="Required">{(p) => <input {...p} />}</Field>)
  expect(screen.getByLabelText('Title').getAttribute('aria-invalid')).toBe('true')
  expect(screen.getByRole('alert').textContent).toBe('Required')
  expect(screen.queryByText('Shown in the list')).toBeNull()
})

test('Choice selects exactly one option and reports it', () => {
  const onChange = vi.fn()
  const options = [{ value: 'a', label: 'Alpha', hint: 'First' }, { value: 'b', label: 'Beta' }]
  render(<Choice label="Letter" options={options} value="a" onChange={onChange} />)
  expect(screen.getAllByRole('radio')).toHaveLength(2)
  expect((screen.getByLabelText(/Alpha/) as HTMLInputElement).checked).toBe(true)
  expect((screen.getByLabelText('Beta') as HTMLInputElement).checked).toBe(false)
  fireEvent.click(screen.getByLabelText('Beta'))
  expect(onChange).toHaveBeenCalledExactlyOnceWith('b')
})

test('Switch exposes role=switch and toggles', () => {
  const onChange = vi.fn()
  render(<Switch label="Notetaker joins Planning" checked={false} onChange={onChange} />)
  const sw = screen.getByRole('switch', { name: 'Notetaker joins Planning' })
  expect(sw.getAttribute('aria-checked')).toBe('false')
  fireEvent.click(sw)
  expect(onChange).toHaveBeenCalledExactlyOnceWith(true)
})

function Demo() {
  const [v, setV] = useState('summary')
  return <Tabs label="Report" value={v} onChange={setV}
    tabs={[{ value: 'summary', label: 'Summary' }, { value: 'transcript', label: 'Transcript' }]} />
}

test('Tabs mark the selected tab and move with the arrow keys', () => {
  render(<Demo />)
  expect(screen.getAllByRole('tab')).toHaveLength(2)
  expect(screen.getByRole('tab', { name: 'Summary' }).getAttribute('aria-selected')).toBe('true')
  fireEvent.keyDown(screen.getByRole('tab', { name: 'Summary' }), { key: 'ArrowRight' })
  expect(screen.getByRole('tab', { name: 'Transcript' }).getAttribute('aria-selected')).toBe('true')
  expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Transcript' }))
  fireEvent.keyDown(screen.getByRole('tab', { name: 'Transcript' }), { key: 'ArrowRight' })
  expect(screen.getByRole('tab', { name: 'Summary' }).getAttribute('aria-selected')).toBe('true')
})

test('ConfirmDialog is a labelled alertdialog, focuses Cancel, closes on Esc, confirms and shows an error', () => {
  const [ok, no] = [vi.fn(), vi.fn()]
  render(<ConfirmDialog title="Delete this?" confirmLabel="Delete" error="boom" onConfirm={ok} onCancel={no}>Gone for good.</ConfirmDialog>)
  const d = screen.getByRole('alertdialog', { name: 'Delete this?' })
  expect(d.getAttribute('aria-describedby')).toBeTruthy()
  expect(document.getElementById(d.getAttribute('aria-describedby')!)!.textContent).toBe('Gone for good.')
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }))
  expect(screen.getByRole('alert').textContent).toBe('boom')
  fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
  expect([ok.mock.calls.length, no.mock.calls.length]).toEqual([1, 0])
  fireEvent.keyDown(document, { key: 'Escape' })
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
  expect([ok.mock.calls.length, no.mock.calls.length]).toEqual([1, 2])
})
