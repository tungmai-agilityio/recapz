// Shared by the documents list and the document page.

/** 1.5 MB, 320 KB, 12 B. */
export function bytesLabel(n: number) {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`
  return `${Number((n / (1024 * 1024)).toFixed(1))} MB`
}

export const addedOn = (ms: number) => new Date(ms).toLocaleDateString([], { year: 'numeric', month: 'short', day: 'numeric' })

export const tone = { indexing: 'signal', indexed: 'good', 'no text found': 'neutral' } as const
export const label = { indexing: 'Indexing', indexed: 'Indexed', 'no text found': 'No text found' } as const
