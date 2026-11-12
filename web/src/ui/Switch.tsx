export function Switch({ label, checked, onChange }:
  { label: string; checked: boolean; onChange: (next: boolean) => void }) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label}
      onClick={() => onChange(!checked)}
      className={`relative h-[18px] w-8 flex-none rounded-full ${checked ? 'bg-primary' : 'bg-hair'}`}>
      <span className={`absolute top-0.5 h-3.5 w-3.5 rounded-full bg-sheet shadow transition-[left] motion-reduce:transition-none ${checked ? 'left-4' : 'left-0.5'}`} />
    </button>
  )
}
