import { Route, Routes } from 'react-router'
import { Shell } from './Shell'
import { Ask } from './pages/Ask'
import { Documents } from './pages/Documents'
import { Live } from './pages/Live'
import { Meetings } from './pages/Meetings'
import { DetectedMeeting } from './pages/Record'
import { Recording } from './pages/Recording'
import { Report } from './pages/Report'
import { Upload } from './pages/Upload'

export default function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<Meetings />} />
        <Route path="/upload" element={<Upload />} />
        <Route path="/ask" element={<Ask />} />
        <Route path="/documents" element={<Documents />} />
        <Route path="/recording" element={<Recording />} />
        <Route path="/meetings/:id" element={<Report />} />
        <Route path="/meetings/:id/live" element={<Live />} />
      </Routes>
      <DetectedMeeting />
    </Shell>
  )
}
