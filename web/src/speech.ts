import { useQuery } from '@tanstack/react-query'
import { getAudioRetention, getMe, getTranscriptRetention, getUsage, Unauthorized } from './api'

/** This month's estimated cost; only asks the server when the signed-in member is an admin. */
export function useUsage() {
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  return useQuery({ queryKey: ['usage'], queryFn: getUsage, enabled: me.data?.role === 'admin' })
}

/** Where recordings are kept and how long their audio stays, for the privacy screen. */
export function useRecordingStorage() {
  const me = useQuery({ queryKey: ['me'], queryFn: getMe, retry: (n, e) => !(e instanceof Unauthorized) && n < 2 })
  const days = useQuery({ queryKey: ['audio-retention'], queryFn: getAudioRetention })
  const meetingDays = useTranscriptRetention().data?.days
  return { where: me.data?.recording_storage, days: days.data?.days, meetingDays, isAdmin: me.data?.role === 'admin' }
}

/** Days a finished meeting is kept before it is deleted; 0 keeps it. */
export function useTranscriptRetention() {
  return useQuery({ queryKey: ['transcript-retention'], queryFn: getTranscriptRetention })
}
