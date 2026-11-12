// The Skald delivery prototype's mod. Once a second it asks `agent-hook --count` how many messages
// wait in the source and shows that under the prompt; while the session is idle, it takes them with
// `agent-hook --take`, which marks them handed over, and starts a turn with their text. A prompt the
// session drops goes back with `agent-hook --release`. The handler and the source come from
// `.skald/mod.json` in the session's directory: { "handler": <agent-hook>, "source": <dir> }.

// Whether a main turn runs, and whether a submit waits for its turn to start.
let running = false
let submitting = false

async function poll($, config) {
  if (submitting) return
  const count = await $.process.run([config.handler, '--count', config.source])
  const waiting = Number(count.stdout.trim()) || 0
  $.ui.status(waiting > 0 ? `${waiting} Skald message(s) waiting` : undefined)
  if (waiting === 0 || running) return
  submitting = true
  try {
    const taken = await $.process.run([config.handler, '--take', config.source])
    if (taken.exitCode !== 0 || taken.stdout.trim() === '') return
    const { ids, text } = JSON.parse(taken.stdout)
    const result = await $.prompt.submit({ text })
    if (result && result.drop) {
      $.ui.log(`skald-mod: the prompt was dropped: ${result.drop}`, { to: 'debug' })
      await $.process.run([config.handler, '--release', config.source, ...ids])
    }
  } finally {
    submitting = false
  }
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    let config
    try {
      config = JSON.parse(await $.fs.read('.skald/mod.json'))
    } catch (error) {
      $.ui.log(`skald-mod: no .skald/mod.json: ${error}`, { to: 'debug' })
      return started
    }
    $.clock.every(1000, () =>
      poll($, config).catch((error) => $.ui.log(`skald-mod: ${error}`, { to: 'debug' })),
    )
    return started
  })

  on('turn.start', async ($, e, next) => {
    if (e.agentId === undefined) running = true
    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    if (e.agentId === undefined) running = false
    return next(e)
  })
}
