import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './i18n'
import { Layout } from './app/layout'
import './styles/globals.css'

const rootEl = document.getElementById('root')
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <Layout />
    </StrictMode>,
  )
}
