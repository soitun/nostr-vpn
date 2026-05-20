import { IMPOSSIBLE, VersionInfo } from '@start9labs/start-sdk'

export const v_4_0_37_0 = VersionInfo.of({
  version: '4.0.37:0',
  releaseNotes: {
    en_US: 'Initial StartOS package for Nostr VPN.',
  },
  migrations: {
    up: async () => {},
    down: IMPOSSIBLE,
  },
})
