/**
 * Stages @ev3-cockpit/client from a compiled EVƎ Cockpit checkout and packs it into
 * .cockpit-client/ev3-cockpit-client.tgz, the tarball package.json depends on until the
 * package is published.
 *
 *   node scripts/stage-cockpit-client.cjs [<cockpit checkout>]
 *
 * The checkout defaults to EV3_COCKPIT_REPO, else ../ev3-cockpit. It must be compiled
 * (`npm run compile` there). Nothing in the checkout is written: the cockpit's own
 * stageClient reads its out/ folder and writes here.
 */
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const ROOT = path.resolve(__dirname, '..');
const OUT = path.join(ROOT, '.cockpit-client');
const TARBALL = 'ev3-cockpit-client.tgz';

function main() {
    const cockpit = path.resolve(process.argv[2] ?? process.env.EV3_COCKPIT_REPO ?? path.join(ROOT, '..', 'ev3-cockpit'));
    const script = path.join(cockpit, 'scripts', 'package-npm.cjs');
    if (!fs.existsSync(script)) {
        throw new Error(`${cockpit} is not an EVƎ Cockpit checkout: ${script} is missing.`);
    }
    const { stageClient } = require(script);
    const root = JSON.parse(fs.readFileSync(path.join(cockpit, 'package.json'), 'utf8'));
    fs.mkdirSync(OUT, { recursive: true });
    const dir = stageClient(OUT, root, path.join(cockpit, 'out'));
    if (!fs.existsSync(path.join(dir, 'client', 'nativeClient.js'))) {
        throw new Error('The staged client has no native client; check out the cockpit branch that carries it and compile it.');
    }

    for (const old of fs.readdirSync(OUT).filter(name => name.endsWith('.tgz'))) {
        fs.rmSync(path.join(OUT, old));
    }
    const packed = spawnSync(`npm pack --pack-destination "${OUT}"`, { cwd: dir, encoding: 'utf8', shell: true });
    if (packed.status !== 0) {
        throw new Error(`npm pack failed:\n${packed.stderr}`);
    }
    const [filename] = fs.readdirSync(OUT).filter(name => name.endsWith('.tgz'));
    fs.renameSync(path.join(OUT, filename), path.join(OUT, TARBALL));
    console.log(`Staged @ev3-cockpit/client ${root.version} from ${cockpit} into ${path.join(OUT, TARBALL)}. Run npm install to take it.`);
}

try {
    main();
} catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
}
