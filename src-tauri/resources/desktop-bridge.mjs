import { createInterface } from 'node:readline'

export const name = 'dsh-desktop-bridge'
export const inject = ['agents', 'appExit']

const sentinel = '@@DSH_DESKTOP@@'
const protocolVersion = 1

function response(requestId, payload) {
  process.stdout.write(`${sentinel}${JSON.stringify({ protocolVersion, requestId, ...payload })}\n`)
}

function protocolError(requestId, error) {
  response(typeof requestId === 'string' ? requestId : '', { ok: false, error })
}

export function parseRequest(line) {
  if (!line.startsWith(sentinel)) return null
  if (Buffer.byteLength(line, 'utf8') > 65536) {
    return { requestId: '', error: 'message-too-large' }
  }
  let request
  try {
    request = JSON.parse(line.slice(sentinel.length))
  } catch {
    return { requestId: '', error: 'invalid-json' }
  }
  if (typeof request?.requestId !== 'string' || request.requestId.length === 0) {
    return { requestId: '', error: 'invalid-request' }
  }
  if (request.protocolVersion !== protocolVersion) {
    return { requestId: request.requestId, error: 'unsupported-protocol' }
  }
  if (typeof request.operation !== 'string') {
    return { requestId: request.requestId, error: 'invalid-operation' }
  }
  return { request }
}

function runningAgents(ctx) {
  return ctx.agents.list().filter((agent) => agent.status === 'running').length
}

export function apply(ctx) {
  let draining = false
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false })

  input.on('line', (line) => {
    const parsed = parseRequest(line)
    if (!parsed) return
    if (parsed.error) {
      protocolError(parsed.requestId, parsed.error)
      return
    }
    const { request } = parsed

    switch (request.operation) {
      case 'status':
        response(request.requestId, {
          ok: true,
          acceptingNewWork: !draining,
          activeWork: runningAgents(ctx),
        })
        break
      case 'beginDrain':
        draining = true
        response(request.requestId, {
          ok: true,
          acceptingNewWork: false,
          activeWork: runningAgents(ctx),
        })
        break
      case 'appExit': {
        const activeWork = runningAgents(ctx)
        if (!draining || activeWork !== 0) {
          response(request.requestId, { ok: false, error: 'drain-required', activeWork })
          break
        }
        response(request.requestId, { ok: true, acceptingNewWork: false, activeWork: 0 })
        ctx.appExit(0)
        break
      }
      default:
        response(request.requestId, { ok: false, error: 'unknown-operation' })
    }
  })

  ctx.effect(() => () => input.close(), 'dsh-desktop-bridge.stdin')
}
