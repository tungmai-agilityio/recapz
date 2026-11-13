import { Route, Routes } from 'react-router'
import { Shell } from './Shell'
import { Ask } from './pages/Ask'
import { Meetings } from './pages/Meetings'
import { Recording } from './pages/Recording'
import { Report } from './pages/Report'

export default function App() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<Meetings />} />
        <Route path="/ask" element={<Ask />} />
        <Route path="/recording" element={<Recording />} />
        <Route path="/meetings/:id" element={<Report />} />
      </Routes>
    </Shell>
  )
}
