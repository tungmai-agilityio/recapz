import { useQuery } from '@tanstack/react-query'
import { getAudioRetention, getMe, Unauthorized } from './api'

/** Name of the provider that receives call and upload audio, from `/api/me` (shares the Shell's query). */
/** Where recordings are kept and how long their audio stays, for the privacy screen. */
export function useRecordingStorage() {
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  const days = useQuery({ queryKey: ['audio-retention'], queryFn: getAudioRetention })
  return { where: me.data?.recording_storage, days: days.data?.days }
}

export function useSpeechProvider(): string {
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  return me.data?.speech_provider ?? 'the speech provider'
}
