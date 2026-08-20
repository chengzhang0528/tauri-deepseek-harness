import { createInterface } from 'node:readline'

export const name = 'dsh-desktop-bridge'
export const inject = ['agents', 'appExit']

const sentinel = '@@DSH_DESKTOP@@'
const protocolVersion = 1

function response(requestId, payload) {
  process.stdout.write(`${sentinel}${JSON.stringify({ protocolVersion, requestId, ...payload })}\n`)
}

function runningAgents(ctx) {
  return ctx.agents.list().filter((agent) => agent.status === 'running').length
}

export function apply(ctx) {
  let draining = false
  const input = createInterface({ input: process.stdin, crlfDelay: Infinity, terminal: false })

  input.on('line', (line) => {
    if (!line.startsWith(sentinel)) return
    if (Buffer.byteLength(line, 'utf8') > 65536) return

    let request
    try {
      request = JSON.parse(line.slice(sentinel.length))
    } catch {
      return
    }
    if (request?.protocolVersion !== protocolVersion || typeof request.requestId !== 'string') return

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
