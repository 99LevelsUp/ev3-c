/**
 * Replays EVƎ Cockpit's "usb-deploy-and-run-ccc" scenario through EVƎ C's own cockpit link
 * (session, brick hold, port), against the native daemon in replay mode. No brick is needed.
 *
 *   1. in the cockpit checkout: npm run compile && npm run daemon:build
 *   2. start the daemon on a test channel:
 *        <cockpit>\daemon\target\debug\ev3-cockpitd.exe --channel t-evec --mode replay
 *          --fixtures <cockpit>\daemon\fixtures\service\usb-deploy-and-run-ccc
 *   3. here: npm run compile && node tests/replay/deploy-and-run.cjs
 *
 * A matched replay answers each recorded call as often as the recording has it, so every
 * run needs a fresh daemon. The paths and the program are the scenario's (its scenario.md).
 */
'use strict';

const path = require('node:path');
const out = path.join(__dirname, '..', '..', 'out');
const { connectCockpit } = require(path.join(out, 'cockpit', 'nativeCockpit.js'));
const { CockpitSession } = require(path.join(out, 'host', 'cockpitSession.js'));

const FOLDER = '/home/root/lms2012/prjs/CockpitFixture';
const PROGRAM = `${FOLDER}/Wait.rbf`;
const WAIT_RBF = 'TEVHTyQAAABtAAEAAAAAABwAAAAAAAAABAAAAIWCQB9AhkAK';
const END_TIMEOUT_MS = 30_000;

function step(text) {
    console.log(`[${new Date().toISOString().slice(11, 23)}] ${text}`);
}

async function main() {
    const channel = process.env.EV3_COCKPIT_CHANNEL ?? 't-evec';
    const session = new CockpitSession(() => connectCockpit({ name: 'EVƎ C replay test', version: '0.0.1', channel }));
    const { client } = await session.ready();
    step(`connected to daemon ${client.daemon.version} (${client.daemon.mode}) on channel ${channel}`);

    const { port } = await session.brick('CCC');
    step(`holding ${port.brickId}`);

    const exists = await port.exists(FOLDER);
    step(`files/exists ${FOLDER}: ${exists}`);

    const states = [];
    let ended;
    const end = new Promise((resolve, reject) => {
        ended = resolve;
        setTimeout(() => reject(new Error(`no end of the program within ${END_TIMEOUT_MS} ms; states: ${JSON.stringify(states)}`)), END_TIMEOUT_MS).unref();
    });
    const subscription = await port.onBrickEvent(event => {
        states.push(event);
        step(`event ${JSON.stringify(event)}`);
        if (event.kind === 'program' && !event.state.running && event.state.runSeq >= 1) {
            ended(event.state);
        }
    });

    const start = await port.deployAndRun([{ brickPath: PROGRAM, source: { data: Buffer.from(WAIT_RBF, 'base64') } }], PROGRAM);
    step(`deployAndRun: ${JSON.stringify(start)}`);
    if (start.outcome !== 'started' || start.runSeq !== 1 || start.uploaded.length !== 1) {
        throw new Error('the start does not match the recording');
    }

    const last = await end;
    step(`program ended: ${JSON.stringify(last)}`);
    subscription.dispose();

    const removed = await port.removePaths([FOLDER], { recursive: true });
    step(`files/removePaths: ${JSON.stringify(removed)}`);
    await client.request('bricks/disconnect', { brick: port.brickId });
    step('disconnected');
    session.dispose();
}

main().then(() => {
    step('PASS');
}, error => {
    console.error(`FAIL: ${error && error.code ? `[${error.code}] ` : ''}${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
}).finally(() => setTimeout(() => process.exit(), 100));
