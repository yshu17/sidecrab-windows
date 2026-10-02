// `claude plugin test plugin`: the session.measure -> limits.json writer.
// The test's own hooks stand for the engine: the kit's env and clock mocks,
// plus fs hooks that answer exists (the pet's folder) and record the writes.
// A hook standing in for an fs call answers `{ value }`, as the kit requires.
import { describe, expect, mock, test } from 'claude-code/testing'
import type { On, SessionRateLimit } from 'claude-code'

// Forward slashes: the kit's env mock drops backslashes (a real APPDATA has
// them; the plugin turns them into slashes either way).
const APPDATA = 'C:/Users/me/AppData/Roaming'
const DIR = 'C:/Users/me/AppData/Roaming/sidecrab'
const NOW_MS = 1_790_000_000_000
const RESETS = '2026-10-02T18:00:00.000Z'

type Engine = { appData?: string | null; dirExists?: boolean } // appData null: unset

/** The engine hands fs hooks absolute, native paths: compare them by slashes. */
const slashed = (p: string) => p.replace(/\\/g, '/')

/** Answers the plugin's env/fs/clock calls; returns the writes it receives. */
function engine(on: On, { appData = APPDATA, dirExists = true }: Engine = {}) {
  const writes: { path: string; text: string }[] = []
  mock.env(on, appData === null ? {} : { APPDATA: appData })
  mock.clock(on, { now: NOW_MS })
  on('fs.exists', ($, e) => ({ value: dirExists && slashed(e.path) === DIR }) as never)
  on('fs.write', ($, e) => {
    writes.push({ path: slashed(e.path), text: e.text })
    return { value: undefined } as never
  })
  on('session.measure', ($, e) => ({ changed: e.changed }))
  return writes
}

const measure = (rateLimits: SessionRateLimit[], changed: ('context' | 'rateLimits' | 'cost')[] = ['rateLimits']) => ({
  context: { window: 1_000_000 },
  rateLimits,
  changed,
})

describe('session.measure -> limits.json', () => {
  test('writes the five-hour window in the pet\'s shape', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([
      { kind: 'seven_day', percentUsed: 12, resetsAt: RESETS },
      { kind: 'five_hour', percentUsed: 41, resetsAt: RESETS },
    ]))
    expect(writes.length).toBe(1)
    expect(writes[0].path).toBe(`${DIR}/limits.json`)
    expect(JSON.parse(writes[0].text)).toEqual({
      fiveHour: { usedPercentage: 41, resetsAt: Math.floor(Date.parse(RESETS) / 1000) },
      source: 'session',
      ts: NOW_MS / 1000,
    })
  })

  test('keeps a fractional percentage', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 23.5, resetsAt: RESETS }]))
    expect(JSON.parse(writes[0].text).fiveHour.usedPercentage).toBe(23.5)
  })

  test('passes the event on', async ($, on) => {
    engine(on)
    const r = await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 1, resetsAt: RESETS }]))
    expect(r).toEqual({ changed: ['rateLimits'] })
  })

  test('writes nothing without a five_hour window', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([{ kind: 'seven_day', percentUsed: 50, resetsAt: RESETS }]))
    expect(writes.length).toBe(0)
  })

  test('writes nothing without resetsAt', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50 }]))
    expect(writes.length).toBe(0)
  })

  test('writes nothing for an unreadable resetsAt', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50, resetsAt: 'soon' }]))
    expect(writes.length).toBe(0)
  })

  test('writes nothing off a subscription (empty rateLimits)', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([]))
    expect(writes.length).toBe(0)
  })

  test('writes nothing when only the context moved', async ($, on) => {
    const writes = engine(on)
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50, resetsAt: RESETS }], ['context']))
    expect(writes.length).toBe(0)
  })

  test('writes nothing without APPDATA', async ($, on) => {
    const writes = engine(on, { appData: null })
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50, resetsAt: RESETS }]))
    expect(writes.length).toBe(0)
  })

  test('never creates the folder when the pet is not installed', async ($, on) => {
    const writes = engine(on, { dirExists: false })
    await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50, resetsAt: RESETS }]))
    expect(writes.length).toBe(0)
  })

  test('a failing write is swallowed and the event still passes', async ($, on) => {
    mock.env(on, { APPDATA })
    mock.clock(on, { now: NOW_MS })
    on('fs.exists', () => ({ value: true }) as never)
    on('fs.write', () => {
      throw new Error('disk full')
    })
    on('session.measure', ($, e) => ({ changed: e.changed }))
    const r = await $.session.measure(measure([{ kind: 'five_hour', percentUsed: 50, resetsAt: RESETS }]))
    expect(r).toEqual({ changed: ['rateLimits'] })
  })
})
