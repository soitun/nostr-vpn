export const DEFAULT_LANG = 'en_US'

const dict = {
  'Starting Nostr VPN': 0,
  'Mesh daemon': 1,
  'The mesh daemon is running': 2,
  'Web Interface': 3,
  'The web interface is ready': 4,
  'The web interface is not ready': 5,
  'Web UI': 6,
  'Open the Nostr VPN control panel': 7,
} as const

export type I18nKey = keyof typeof dict
export type LangDict = Record<(typeof dict)[I18nKey], string>
export default dict
