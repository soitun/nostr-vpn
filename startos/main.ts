import { i18n } from './i18n'
import { sdk } from './sdk'
import { dataMount, uiPort } from './utils'

const commonEnv = {
  HOME: '/data/home',
  XDG_CONFIG_HOME: '/data/config',
  RUST_LOG: 'info',
}

export const main = sdk.setupMain(async ({ effects }) => {
  console.info(i18n('Starting Nostr VPN'))

  const daemonSub = await sdk.SubContainer.of(
    effects,
    { imageId: 'app', sharedRun: true },
    dataMount,
    'nostr-vpn-daemon',
  )
  const webSub = await sdk.SubContainer.of(
    effects,
    { imageId: 'app' },
    dataMount,
    'nostr-vpn-web',
  )

  return sdk.Daemons.of(effects)
    .addOneshot('prepare-data', {
      subcontainer: daemonSub,
      exec: {
        command: ['mkdir', '-p', '/data/home', '/data/config/nvpn'],
        user: 'root',
      },
      requires: [],
    })
    .addDaemon('daemon', {
      subcontainer: daemonSub,
      exec: {
        command: [
          '/usr/local/bin/nvpn',
          'daemon',
          '--paused',
          '--config',
          '/data/config/nvpn/config.toml',
        ],
        env: commonEnv,
      },
      ready: {
        display: i18n('Mesh daemon'),
        fn: async () => ({
          result: 'success' as const,
          message: i18n('The mesh daemon is running'),
        }),
      },
      requires: ['prepare-data'],
    })
    .addDaemon('web', {
      subcontainer: webSub,
      exec: {
        command: [
          '/usr/local/bin/nvpn-web',
          '--listen',
          `0.0.0.0:${uiPort}`,
          '--config',
          '/data/config/nvpn/config.toml',
        ],
        env: {
          ...commonEnv,
          NVPN_CLI_PATH: '/usr/local/bin/nvpn',
          NVPN_DAEMON_STATUS_MODE: 'state-file',
          NVPN_EXTERNAL_DAEMON: 'true',
        },
      },
      ready: {
        display: i18n('Web Interface'),
        fn: () =>
          sdk.healthCheck.checkWebUrl(
            effects,
            `http://localhost:${uiPort}/api/health`,
            {
              successMessage: i18n('The web interface is ready'),
              errorMessage: i18n('The web interface is not ready'),
            },
          ),
      },
      requires: ['daemon'],
    })
})
