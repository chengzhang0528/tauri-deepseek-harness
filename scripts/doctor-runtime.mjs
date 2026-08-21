import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'

const MAX_OUTPUT_BYTES = 64 * 1024
const MAX_PAGE_BYTES = 2 * 1024 * 1024
const BRIDGE_SENTINEL = '@@DSH_DESKTOP@@'
const BRIDGE_PROTOCOL = 1

function parseArgs(argv) {
  const values = new Map()
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index]
    if (!value.startsWith('--')) throw new Error(`unknown argument ${value}`)
    const key = value.slice(2)
    if (key === 'json') {
      values.set(key, true)
      continue
    }
    const next = argv[index + 1]
    if (!next || next.startsWith('--')) throw new Error(`missing value for --${key}`)
    values.set(key, next)
    index += 1
  }
  return values
}

function required(values, name) {
  const value = values.get(name)
  if (!value) throw new Error(`missing --${name}`)
  return resolve(value)
}

function optionalPath(values, name, fallback) {
  const value = values.get(name)
  return resolve(value ?? fallback)
}

async function readMetadata(path) {
  const metadata = JSON.parse(await readFile(path, 'utf8'))
  if (metadata.schema !== 1 || metadata.platform !== 'windows' || metadata.arch !== 'x64') {
    throw new Error('runtime metadata has an unsupported schema or target')
  }
  for (const [name, value] of Object.entries(metadata)) {
    if (['schema', 'platform', 'arch'].includes(name)) continue
    if (!value?.version || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(value.version)) {
      throw new Error(`runtime metadata version is invalid for ${name}`)
    }
  }
  return metadata
}

function findFile(root, candidates) {
  for (const relative of candidates) {
    const path = join(root, relative)
    if (existsSync(path)) return path
  }
  throw new Error(`required runtime file is missing: ${candidates[0]}`)
}

function run(program, args, cwd, timeoutMs = 30_000) {
  return new Promise((resolveResult, reject) => {
    const child = spawn(program, args, {
      cwd,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let output = ''
    const append = (chunk) => {
      output += chunk.toString('utf8')
      if (Buffer.byteLength(output, 'utf8') > MAX_OUTPUT_BYTES) {
        output = output.slice(-MAX_OUTPUT_BYTES)
      }
    }
    child.stdout.on('data', append)
    child.stderr.on('data', append)
    const timer = setTimeout(() => {
      child.kill()
      reject(new Error(`${program} timed out after ${timeoutMs}ms`))
    }, timeoutMs)
    child.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once('exit', (code, signal) => {
      clearTimeout(timer)
      resolveResult({ code, signal, output })
    })
  })
}

async function readBounded(response) {
  if (!response.body) return Buffer.alloc(0)
  const reader = response.body.getReader()
  const chunks = []
  let size = 0
  while (true) {
    const next = await reader.read()
    if (next.done) break
    size += next.value.byteLength
    if (size > MAX_PAGE_BYTES) {
      await reader.cancel()
      throw new Error('dsh readiness page exceeds the client maximum size')
    }
    chunks.push(Buffer.from(next.value))
  }
  return Buffer.concat(chunks)
}

async function waitForReady(lines, root, node, cli, bridgePatch, dshHome) {
  const deadline = Date.now() + 30_000
  const child = spawn(node, [cli, 'web', '--patch', bridgePatch, '--port', '0', '--no-open'], {
    cwd: root,
    env: { ...process.env, DSH_HOME: dshHome },
    windowsHide: true,
    stdio: ['pipe', 'pipe', 'pipe'],
  })
  const append = (chunk) => {
    const text = chunk.toString('utf8')
    lines.push(...text.split(/\r?\n/).filter(Boolean))
    while (lines.length > 256) lines.shift()
  }
  child.stdout.on('data', append)
  child.stderr.on('data', append)
  try {
    while (Date.now() < deadline) {
      const line = lines.find((value) => /https?:\/\/(?:127\.0\.0\.1|localhost):\d+/.test(value))
      const match = line?.match(/http:\/\/(?:127\.0\.0\.1|localhost):(\d+)/)
      if (match) {
        const url = `http://127.0.0.1:${match[1]}/`
        const response = await fetch(url, { signal: AbortSignal.timeout(2_000) }).catch(() => null)
        if (response?.ok) {
          const body = (await readBounded(response)).toString('utf8')
          if (body.includes('window.__DSH_BOOT__')) return { child, url }
        }
      }
      await new Promise((resolveDelay) => setTimeout(resolveDelay, 100))
    }
    throw new Error('dsh web did not expose a verified ready page before timeout')
  } catch (error) {
    child.kill()
    throw error
  }
}

function requestBridge(child, lines, operation) {
  const requestId = `${operation}-${Date.now()}`
  child.stdin.write(`${BRIDGE_SENTINEL}${JSON.stringify({ protocolVersion: BRIDGE_PROTOCOL, requestId, operation })}\n`)
  return new Promise((resolveResult, reject) => {
    const deadline = Date.now() + 2_000
    const timer = setInterval(() => {
      const response = lines
        .map((line) => line.startsWith(BRIDGE_SENTINEL) ? line.slice(BRIDGE_SENTINEL.length) : null)
        .filter(Boolean)
        .map((line) => { try { return JSON.parse(line) } catch { return null } })
        .find((value) => value?.requestId === requestId)
      if (response) {
        clearInterval(timer)
        if (!response.ok) reject(new Error(`bridge ${operation} failed: ${response.error ?? 'unknown-error'}`))
        else resolveResult(response)
      } else if (Date.now() >= deadline) {
        clearInterval(timer)
        reject(new Error(`bridge ${operation} timed out: ${lines.slice(-12).join('\n')}`))
      }
    }, 50)
  })
}

async function main() {
  const values = parseArgs(process.argv.slice(2))
  const root = required(values, 'root')
  const metadataPath = optionalPath(values, 'metadata', join(root, 'runtime-versions.windows-x64.json'))
  const metadata = await readMetadata(metadataPath)
  const node = findFile(root, ['node.exe', 'node/node.exe', 'runtime/node.exe', 'bin/node.exe'])
  const ripgrep = findFile(root, ['rg.exe', 'ripgrep.exe', 'bin/rg.exe'])
  const cli = findFile(root, [
    'node_modules/@deepseek-ai/dsh/lib/bin.js',
    'node_modules/@deepseek-ai/dsh/dist/cli.js',
    'node_modules/@deepseek-ai/deepseek-harness/dist/cli.js',
    'dsh/cli.js',
  ])
  const bridgePatch = findFile(root, ['desktop-bridge.patch.yml'])
  findFile(root, ['desktop-bridge.mjs'])

  const checks = []
  for (const [name, program, args] of [
    ['node', node, ['--version']],
    ['ripgrep', ripgrep, ['--version']],
    ['dsh', node, [cli, '--version']],
  ]) {
    const result = await run(program, args, root)
    if (result.code !== 0) throw new Error(`${name} doctor failed: ${result.output.trim()}`)
    checks.push({ name, output: result.output.trim().split(/\r?\n/)[0] })
  }

  const nodeVersion = checks.find((check) => check.name === 'node')?.output.replace(/^v/, '')
  if (nodeVersion !== metadata.node.version) {
    throw new Error(`node version ${nodeVersion} does not match pinned ${metadata.node.version}`)
  }
  const ripgrepVersion = checks.find((check) => check.name === 'ripgrep')?.output.match(/(\d+\.\d+\.\d+)/)?.[1]
  if (ripgrepVersion !== metadata.ripgrep.version) {
    throw new Error(`ripgrep version ${ripgrepVersion ?? 'unknown'} does not match pinned ${metadata.ripgrep.version}`)
  }

  const dshPackage = JSON.parse(await readFile(join(root, 'node_modules', '@deepseek-ai', 'dsh', 'package.json'), 'utf8'))
  if (dshPackage.version !== metadata.dsh.version) {
    throw new Error(`dsh version ${dshPackage.version} does not match pinned ${metadata.dsh.version}`)
  }

  const pnpm = findFile(root, ['node_modules/pnpm/bin/pnpm.cjs', 'pnpm.exe'])
  const pnpmResult = pnpm.endsWith('.cjs')
    ? await run(node, [pnpm, '--version'], root)
    : await run(pnpm, ['--version'], root)
  if (pnpmResult.code !== 0) throw new Error(`pnpm doctor failed: ${pnpmResult.output.trim()}`)
  const pnpmVersion = pnpmResult.output.trim().split(/\r?\n/)[0]
  if (pnpmVersion !== metadata.pnpm.version) {
    throw new Error(`pnpm version ${pnpmVersion} does not match pinned ${metadata.pnpm.version}`)
  }
  checks.push({ name: 'pnpm', output: pnpmVersion })

  const nativeModules = detectNativeModules(root)
  const nativeResult = await run(node, ['-e', `for (const name of ${JSON.stringify(nativeModules)}) require(name)`], root)
  if (nativeResult.code !== 0) throw new Error(`native module doctor failed: ${nativeResult.output.trim()}`)
  checks.push({ name: 'native-modules', output: nativeModules.length === 0 ? 'none' : nativeModules.join(', ') })

  const doctorHome = await mkdtemp(join(tmpdir(), 'dsh-desktop-doctor-'))
  try {
    const lines = []
    const { child, url } = await waitForReady(lines, root, node, cli, bridgePatch, doctorHome)
    try {
      await requestBridge(child, lines, 'status')
      await requestBridge(child, lines, 'beginDrain')
      await requestBridge(child, lines, 'appExit')
      checks.push({ name: 'web', output: url })
    } finally {
      child.kill()
    }
  } finally {
    await rm(doctorHome, { recursive: true, force: true })
  }

  const result = { root, metadata: metadataPath, checks, nativeModules }
  if (values.get('json')) process.stdout.write(`${JSON.stringify(result)}\n`)
  else process.stdout.write(`${checks.map((check) => `${check.name}: ${check.output}`).join('\n')}\n`)
}

function detectNativeModules(root) {
  const packagePath = join(root, 'node_modules', 'package.json')
  if (!existsSync(packagePath)) return []
  const names = ['node-pty', 'koffi', 'sharp'].filter((name) => existsSync(join(root, 'node_modules', name)))
  return names
}

try {
  await main()
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`runtime doctor failed: ${message}\n`)
  process.exitCode = 1
}
