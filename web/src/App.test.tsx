import { cleanup, render, screen } from '@testing-library/react'
import { MemoryRouter } from 'react-router'
import { afterEach, expect, test } from 'vitest'
import App from './App'

afterEach(cleanup)

test('renders the app shell', () => {
  render(<MemoryRouter><App /></MemoryRouter>)
  expect(screen.getByText('Recapz')).toBeTruthy()
})
