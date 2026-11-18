import { useQuery } from '@tanstack/react-query'
import { getMe, Unauthorized } from './api'

/** Name of the provider that receives call and upload audio, from `/api/me` (shares the Shell's query). */
export function useSpeechProvider(): string {
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  return me.data?.speech_provider ?? 'the speech provider'
}
