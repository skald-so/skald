// Skald's mod (docs/protocol/delivery.md, "Device to agent"). Once a second it runs
// `skald hook claude --mod` from PATH in the session's folder, as the agent of the session's hooks,
// shows how many messages wait under the prompt and, while no turn runs, submits the prompt that
// skald gives: a fixed request to call inbox, never a message's text, as the prompt is the user's.

// Whether a main turn runs, and whether a poll runs.
let running = false
let polling = false

async function poll($) {
  if (polling) return
  polling = true
  try {
    const args = ['skald', 'hook', 'claude', '--mod']
    if (!running) args.push('--idle')
    const polled = await $.process.run(args)
    if (polled.exitCode !== 0) return
    const { waiting, prompt } = JSON.parse(polled.stdout)
    $.ui.status(waiting > 0 ? `${waiting} Skald message(s) waiting` : undefined)
    if (!prompt || running) return
    const result = await $.prompt.submit({ text: prompt })
    if (result && result.drop) $.ui.log(`skald: the prompt was dropped: ${result.drop}`, { to: 'debug' })
  } finally {
    polling = false
  }
}

export function register(on) {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    $.clock.every(1000, () => poll($).catch((error) => $.ui.log(`skald: ${error}`, { to: 'debug' })))
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
