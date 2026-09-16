import assert from 'node:assert/strict'
import test from 'node:test'
import { parseRequest, handleRequest } from '../src-tauri/resources/desktop-bridge.mjs'

test('bridge rejects malformed and unsupported requests', () => {
  assert.equal(parseRequest('ordinary log'), null)
  assert.equal(parseRequest('@@DSH_DESKTOP@@{').error, 'invalid-json')
  assert.equal(parseRequest('@@DSH_DESKTOP@@{"protocolVersion":1,"requestId":"r","operation":"status"}').error, 'unsupported-protocol')
  assert.equal(parseRequest(`@@DSH_DESKTOP@@${'x'.repeat(70000)}`).error, 'message-too-large')
  assert.equal(parseRequest('@@DSH_DESKTOP@@{"protocolVersion":2,"requestId":"r"}').error, 'invalid-operation')
})
test('zero running agents does not imply no pending work or completed cleanup', () => {
  const status = handleRequest({ agents: { list: () => [{ status: 'waiting' }] } }, 'status')
  assert.deepEqual(status, { ok: true, observedRunningAgents: 0, acceptingNewWork: null, pendingWork: null, cleanupComplete: null })
})
test('unavailable agent list remains unknown', () => {
  assert.equal(handleRequest({}, 'status').observedRunningAgents, null)
})
test('unsupported drain and exit never invoke the upstream exit hook', () => {
  let called = false
  const ctx = { appExit() { called = true } }
  for (const op of ['beginDrain', 'appExit']) assert.deepEqual(handleRequest(ctx, op), { ok: false, error: 'exit-evidence-unavailable' })
  assert.equal(called, false)
})
