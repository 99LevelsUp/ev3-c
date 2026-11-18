/**
 * Throwaway hardware probes for the EVƎ C run contract, driven through EVƎ Cockpit's API.
 *
 * Needs: one brick connected, EVƎ Cockpit with "ev3-cockpit.accessMode": "full" (the shell
 * probes need it), and the probe binaries built into ../out (probe_hello, probe_sup, spike.rbf).
 * Every step is written to the "EVƎ C spike" output channel and to ~/ev3-c-spike.txt.
 * The person at the brick answers the questions about the motor, the screen, and Back.
 */
'use strict';

const vscode = require('vscode');
const fs = require('fs');
const os = require('os');
const path = require('path');

const DIR = '/home/root/lms2012/prjs/spk';
const OUT = path.join(__dirname, '..', 'out');
const LOG_FILE = path.join(os.homedir(), 'ev3-c-spike.txt');

let out;
let consumer;
let brick;

function line(text) {
    const stamped = `${new Date().toISOString().slice(11, 23)} ${text}`;
    out.appendLine(stamped);
    fs.appendFileSync(LOG_FILE, `${stamped}\n`);
}

const describe = (error) => `${error?.code ? `[${error.code}] ` : ''}${error?.message ?? String(error)}`;
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const text = (bytes) => Buffer.from(bytes).toString('utf8');

async function step(name, fn) {
    const started = Date.now();
    try {
        const detail = await fn();
        line(`ok    ${name}${detail ? ` — ${detail}` : ''} (${Date.now() - started} ms)`);
    } catch (error) {
        line(`FAIL  ${name} — ${describe(error)} (${Date.now() - started} ms)`);
    }
}

async function ask(question, ...answers) {
    const answer = await vscode.window.showInformationMessage(question, { modal: true }, ...answers);
    line(`ask   ${question} → ${answer ?? '(dismissed)'}`);
    return answer;
}

/** Runs a short shell command with its output redirected to a file, then reads that file. */
async function shell(name, command) {
    await step(`shell: ${name}`, async () => {
        const file = `${DIR}/p.txt`;
        const result = await consumer.runShell(brick, `(${command}) > ${file} 2>&1`);
        const output = text(await consumer.readFile(brick, file)).trim();
        line(`      status=${result.status}`);
        for (const l of output.split('\n')) { line(`      | ${l}`); }
    });
}

async function readLog(label) {
    await step(`sup.log after ${label}`, async () => {
        const log = text(await consumer.readFile(brick, `${DIR}/sup.log`)).trim();
        for (const l of log.split('\n')) { line(`      | ${l}`); }
    });
}

async function setOpts(opts) {
    await consumer.writeFile(brick, `${DIR}/opts`, Buffer.from(opts));
}

/** Polls the slot state and a small read for a while, to see whether the VM stays responsive. */
async function responsiveness(seconds) {
    const samples = [];
    for (let i = 0; i < seconds; i++) {
        const started = Date.now();
        const state = await consumer.isProgramRunning(brick);
        await consumer.readFile(brick, `${DIR}/opts`);
        samples.push(`${state.running ? 'R' : '-'}${Date.now() - started}ms`);
        await wait(1000);
    }
    return samples.join(' ');
}

async function launcherRuns() {
    const launcher = `${DIR}/spk.rbf`;

    // Run 1: stop from the cockpit.
    await setOpts('');
    await step('run 1: start launcher', async () => JSON.stringify(await consumer.runProgram(brick, launcher, { replace: true })));
    await step('run 1: responsiveness while running (state + 1 read per second)', () => responsiveness(10));
    await ask('Run 1: is motor A turning?', 'Yes', 'No');
    await step('run 1: stopProgram', async () => JSON.stringify(await consumer.stopProgram(brick)));
    await wait(2000);
    await ask('Run 1 after Stop: has motor A stopped and stayed stopped?', 'Yes', 'No');
    await readLog('run 1');

    // Run 2: Back takeover.
    await setOpts('back');
    await step('run 2: start launcher', async () => JSON.stringify(await consumer.runProgram(brick, launcher, { replace: true })));
    await wait(2000);
    await ask('Run 2: press Back BRIEFLY now, then click OK.', 'OK');
    await step('run 2: still running after a short press?', async () => JSON.stringify(await consumer.isProgramRunning(brick)));
    await ask('Run 2: now HOLD Back for about 2 s, then click OK.', 'OK');
    await step('run 2: still running after a long press?', async () => JSON.stringify(await consumer.isProgramRunning(brick)));
    await ask('Run 2: has motor A stopped?', 'Yes', 'No');
    await readLog('run 2');

    // Run 3: what the screen shows while the slot runs.
    await setOpts('lcd');
    await step('run 3: start launcher', async () => JSON.stringify(await consumer.runProgram(brick, launcher, { replace: true })));
    await wait(3000);
    await ask('Run 3: what does the brick screen show?', 'Striped pattern (our output)', 'The brick UI', 'Flickers between both');
    await step('run 3: stopProgram', async () => JSON.stringify(await consumer.stopProgram(brick)));
    await wait(2000);
    await readLog('run 3');

    // Run 4: the supervisor dies while the program drives the motor.
    await setOpts('');
    await step('run 4: start launcher', async () => JSON.stringify(await consumer.runProgram(brick, launcher, { replace: true })));
    await wait(2000);
    await step('run 4: kill -9 the supervisor', async () => {
        const result = await consumer.runShell(brick, `kill -9 $(cat ${DIR}/.run)`);
        return `status=${result.status}`;
    });
    await wait(1500);
    await step('run 4: slot state after the supervisor died', async () => JSON.stringify(await consumer.isProgramRunning(brick)));
    await ask('Run 4: is motor A turning now? (The orphaned program may restart it.)', 'Turning', 'Stopped');
    await step('run 4: clean up the orphan', async () => `status=${(await consumer.runShell(brick, 'killall probe_sup')).status}`);
    await step('run 4: emergencyStop', async () => JSON.stringify(await consumer.emergencyStop(brick)));

    // Run 5: the program ends on its own after about 5 s.
    await setOpts('short');
    await step('run 5: start launcher', async () => JSON.stringify(await consumer.runProgram(brick, launcher, { replace: true })));
    await wait(8000);
    await step('run 5: slot state after the program ended', async () => JSON.stringify(await consumer.isProgramRunning(brick)));
    await readLog('run 5');
}

async function runSpike() {
    out = out ?? vscode.window.createOutputChannel('EVƎ C spike');
    out.show(true);
    fs.appendFileSync(LOG_FILE, `\n==== EVƎ C brick spike ${new Date().toISOString()}\n`);

    const cockpit = vscode.extensions.getExtension('99LevelsUp.ev3-cockpit');
    const api = await cockpit.activate();
    consumer = consumer ?? api.registerConsumer('ev3-c-brick-spike');
    line(`cockpit apiVersion=${api.apiVersion} accessMode=${consumer.getAccessMode()}`);
    if (consumer.getAccessMode() !== 'full') {
        line('Set "ev3-cockpit.accessMode": "full" for the spike, then run it again.');
        return;
    }
    brick = consumer.getActiveBrickId();
    if (!brick) {
        line('Connect a brick and make it active, then run the spike again.');
        return;
    }
    await step('brick info', async () => JSON.stringify(await consumer.getBrickInfo(brick)));

    await step('make the spike folder', () => consumer.makeDirectory(brick, DIR));
    for (const [local, remote] of [['probe_hello', 'probe_hello'], ['probe_sup', 'probe_sup'], ['spike.rbf', 'spk.rbf']]) {
        await step(`upload ${remote}`, async () => {
            const bytes = fs.readFileSync(path.join(OUT, local));
            await consumer.writeFile(brick, `${DIR}/${remote}`, bytes);
            return `${bytes.length} B`;
        });
    }
    await setOpts('');

    await shell('kernel', 'cat /proc/version');
    await shell('libc', 'ls -l /lib/libc* /lib/ld*');
    await shell('mounts', 'cat /proc/mounts');
    await shell('free space', 'df');
    await shell('projects folder', 'ls -la /home/root/lms2012');
    await shell('uploaded file modes', `ls -la ${DIR}`);
    await shell('clock', 'date; cat /proc/uptime');
    await shell('hello probe', `${DIR}/probe_hello; echo exit=$?`);

    await launcherRuns();

    line('Spike finished. Note the clock above; for the battery-clock check, reboot the brick and run "date" again.');
}

function activate(context) {
    context.subscriptions.push(vscode.commands.registerCommand('ev3-c-brick-spike.run', () => runSpike()));
}

function deactivate() {}

module.exports = { activate, deactivate };
