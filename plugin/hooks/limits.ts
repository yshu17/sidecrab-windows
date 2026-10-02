// The 5-hour limit straight from the Claude Code engine: `session.measure`
// fires after each turn and whenever a rate-limit window moves a whole point,
// with the same figures the status line reads. Written to the pet's
// limits.json as source "session"; the pet then skips its own OAuth polling
// while this value is fresh (see src-tauri/src/usage_api.rs).
import type { Register } from 'claude-code'

export const register: Register = on => {
  on('session.measure', async ($, e, next) => {
    try {
      if (e.changed.includes('rateLimits')) {
        const five = e.rateLimits.find(r => r.kind === 'five_hour')
        const appData = await $.env.get('APPDATA')
        if (five && five.resetsAt && appData) {
          const dir = `${appData.replace(/\\/g, '/')}/sidecrab`
          if (await $.fs.exists(dir)) {            // never create the folder when the pet isn't installed
            const resets = Math.floor(Date.parse(five.resetsAt) / 1000)
            if (Number.isFinite(resets)) {
              const now = Math.floor((await $.clock.now()) / 1000)
              await $.fs.write(`${dir}/limits.json`, JSON.stringify(
                { fiveHour: { usedPercentage: five.percentUsed, resetsAt: resets }, source: 'session', ts: now }))
            }
          }
        }
      }
    } catch { /* ignore: the pet falls back to its other sources */ }
    return next(e)
  })
}
