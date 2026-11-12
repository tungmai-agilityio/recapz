import { useState } from 'react'
import { getMode, getSkin, setMode, setSkin, type Mode, type Skin } from './theme'

const group = 'flex overflow-hidden rounded-md border border-hair'
const btn = (on: boolean) => `px-2.5 py-1 text-xs ${on ? 'bg-ink text-paper' : 'bg-sheet text-mute'}`

export function ThemeSwitch() {
  const [skin, setS] = useState<Skin>(getSkin)
  const [mode, setM] = useState<Mode>(getMode)
  const skins: [Skin, string][] = [['day', 'Daylight'], ['ink', 'Ink']]
  const modes: [Mode, string][] = [['light', '☀ Light'], ['dark', '☾ Dark'], ['system', 'Auto']]
  return (
    <div className="ml-auto flex flex-wrap gap-2">
      <div role="group" aria-label="Theme" className={group}>
        {skins.map(([k, label]) => (
          <button key={k} aria-pressed={skin === k} className={btn(skin === k)}
            onClick={() => { setSkin(k); setS(k) }}>{label}</button>
        ))}
      </div>
      <div role="group" aria-label="Light or dark mode" className={group}>
        {modes.map(([k, label]) => (
          <button key={k} aria-pressed={mode === k} className={btn(mode === k)}
            onClick={() => { setMode(k); setM(k) }}>{label}</button>
        ))}
      </div>
    </div>
  )
}
