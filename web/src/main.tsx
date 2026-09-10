import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './i18n'
import { Layout } from './app/layout'
import { wsClient } from './lib/wsClient'
import './styles/globals.css'

wsClient.init()

const rootEl = document.getElementById('root')
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <Layout />
    </StrictMode>,
  )
}
