import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

import { parseRequest } from '../src-tauri/resources/desktop-bridge.mjs'

test('desktop bridge returns stable protocol errors', () => {
  assert.deepEqual(parseRequest('ordinary dsh log'), null)
  assert.equal(parseRequest('@@DSH_DESKTOP@@{').error, 'invalid-json')
  assert.equal(
    parseRequest('@@DSH_DESKTOP@@{"protocolVersion":99,"requestId":"r","operation":"status"}').error,
    'unsupported-protocol',
  )
  assert.equal(
    parseRequest('@@DSH_DESKTOP@@{"protocolVersion":1,"requestId":"r"}').error,
    'invalid-operation',
  )
  assert.equal(parseRequest(`@@DSH_DESKTOP@@${'x'.repeat(65_000)}`).error, 'invalid-json')
  assert.equal(parseRequest(`@@DSH_DESKTOP@@${'x'.repeat(70_000)}`).error, 'message-too-large')
})

test('desktop bridge accepts only the fixed request shape', () => {
  assert.deepEqual(
    parseRequest('@@DSH_DESKTOP@@{"protocolVersion":1,"requestId":"r","operation":"status"}'),
    { request: { protocolVersion: 1, requestId: 'r', operation: 'status' } },
  )
})

test('runtime doctor requires the pinned native module set when present', async () => {
  const doctor = await readFile(new URL('./doctor-runtime.mjs', import.meta.url), 'utf8')
  for (const name of ['node-pty', 'koffi', 'sharp']) {
    assert.match(doctor, new RegExp(`['"]${name}['"]`))
  }
})

test('desktop bridge patch inserts a new root loader entry', async () => {
  const patch = await readFile(new URL('../src-tauri/resources/desktop-bridge.patch.yml', import.meta.url), 'utf8')
  assert.deepEqual(patch.trimEnd().split(/\r?\n/), [
    '- insert:',
    '    - id: dsh-desktop-bridge',
    '      name: __DSH_DESKTOP_BRIDGE_MODULE__',
  ])
})
