import test from 'node:test'
import assert from 'node:assert/strict'

import {
  readStartosSourceVersion,
  resolveStartosTarget,
  startosReleaseAssetName,
  validateStartosManifest,
} from './startos-release.mjs'

test('resolveStartosTarget accepts make targets and architecture names', () => {
  assert.deepEqual(resolveStartosTarget('x86'), {
    arch: 'x86_64',
    makeTarget: 'x86',
  })
  assert.deepEqual(resolveStartosTarget('aarch64'), {
    arch: 'aarch64',
    makeTarget: 'arm',
  })
  assert.throws(() => resolveStartosTarget('riscv'), /Unsupported StartOS target/)
})

test('startosReleaseAssetName includes the release tag and architecture', () => {
  assert.equal(
    startosReleaseAssetName('4.0.97', 'x86_64'),
    'nostr-vpn-v4.0.97-startos-x86_64.s9pk',
  )
  assert.equal(
    startosReleaseAssetName('v4.0.97', 'aarch64'),
    'nostr-vpn-v4.0.97-startos-aarch64.s9pk',
  )
})

test('readStartosSourceVersion reads the SDK version graph source', () => {
  assert.equal(
    readStartosSourceVersion("export const currentVersion = VersionInfo.of({\n  version: '4.0.97:0',\n})\n"),
    '4.0.97:0',
  )
})

test('validateStartosManifest requires the release version and target image', () => {
  const manifest = {
    id: 'nostr-vpn',
    version: '4.0.97:0',
    images: [{ id: 'app', arch: ['x86_64'] }],
  }

  assert.doesNotThrow(() =>
    validateStartosManifest(manifest, { arch: 'x86_64', tag: 'v4.0.97' }),
  )
  assert.throws(
    () => validateStartosManifest(manifest, { arch: 'aarch64', tag: 'v4.0.97' }),
    /does not contain aarch64/,
  )
  assert.throws(
    () => validateStartosManifest(manifest, { arch: 'x86_64', tag: 'v4.0.98' }),
    /version 4\.0\.97:0 does not match release v4\.0\.98/,
  )
})
