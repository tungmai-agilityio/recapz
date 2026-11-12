export type Skin = 'day' | 'ink'
export type Mode = 'light' | 'dark' | 'system'

const dark = () => window.matchMedia('(prefers-color-scheme: dark)')
const read = (key: string) => { try { return localStorage.getItem(key) } catch { return null } }
const write = (key: string, v: string) => { try { localStorage.setItem(key, v) } catch { /* private mode */ } }

export const getSkin = (): Skin => (read('recapz-skin') === 'ink' ? 'ink' : 'day')
export const getMode = (): Mode => {
  const m = read('recapz-mode')
  return m === 'dark' || m === 'system' ? m : 'light'
}

/** Writes data-skin and data-theme on <html>; "Auto" resolves to the system preference. */
export function applyTheme() {
  const root = document.documentElement
  root.dataset.skin = getSkin()
  root.dataset.theme = getMode() === 'system' ? (dark().matches ? 'dark' : 'light') : getMode()
}

export function setSkin(s: Skin) { write('recapz-skin', s); applyTheme() }
export function setMode(m: Mode) { write('recapz-mode', m); applyTheme() }

export function initTheme() {
  applyTheme()
  dark().addEventListener('change', applyTheme)
}
