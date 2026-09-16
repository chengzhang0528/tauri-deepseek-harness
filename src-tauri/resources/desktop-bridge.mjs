import { createInterface } from 'node:readline'

export const name = 'dsh-desktop-bridge'
export const inject = ['agents']
const sentinel = '@@DSH_DESKTOP@@'
const protocolVersion = 2
const maxBytes = 65536

export function parseRequest(line) {
  if (!line.startsWith(sentinel)) return null
  if (Buffer.byteLength(line, 'utf8') > maxBytes) return { requestId: '', error: 'message-too-large' }
  let request
  try { request = JSON.parse(line.slice(sentinel.length)) }
  catch { return { requestId: '', error: 'invalid-json' } }
  if (typeof request?.requestId !== 'string' || !request.requestId || request.requestId.length > 128) return { requestId: '', error: 'invalid-request' }
  if (request.protocolVersion !== protocolVersion) return { requestId: request.requestId, error: 'unsupported-protocol' }
  if (typeof request.operation !== 'string') return { requestId: request.requestId, error: 'invalid-operation' }
  return { request }
}

export function handleRequest(ctx, operation) {
  switch (operation) {
    case 'status': {
      let observedRunningAgents = null
      try { observedRunningAgents = ctx.agents.list().filter(agent => agent.status === 'running').length } catch { /* unavailable */ }
      // The inspected official API provides no aggregate admission, pending-work,
      // or completed-disposal evidence. A running-agent count cannot stand in for it.
      return { ok: true, observedRunningAgents, acceptingNewWork: null, pendingWork: null, cleanupComplete: null }
    }
    case 'beginDrain':
    case 'appExit':
      return { ok: false, error: 'exit-evidence-unavailable' }
    default: return { ok: false, error: 'unknown-operation' }
  }
}

export function apply(ctx) {
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false })
  input.on('line', line => {
    const parsed = parseRequest(line)
    if (!parsed) return
    const requestId = parsed.error ? parsed.requestId : parsed.request.requestId
    const payload = parsed.error ? { ok: false, error: parsed.error } : handleRequest(ctx, parsed.request.operation)
    process.stdout.write(`${sentinel}${JSON.stringify({ protocolVersion, requestId, ...payload })}\n`)
  })
  ctx.effect(() => () => input.close(), 'dsh-desktop-bridge.stdin')
}
