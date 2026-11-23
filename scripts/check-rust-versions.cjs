'use strict';
/**
 * Checks that ev3-c builds with the crate versions EVƎ Cockpit builds with: every crate both
 * Cargo.lock files name must resolve, in ev3-c, to versions the cockpit's lock has too. The two
 * projects are this close, so a library is one version in both, and the cockpit leads.
 *
 *   node scripts/check-rust-versions.cjs [<cockpit Cargo.lock>]
 *
 * Without an argument it reads the cockpit's daemon/Cargo.lock at the revision in
 * backend/cockpit.rev from GitHub. A crate only one project uses is not compared.
 */
const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const REPO = '99LevelsUp/ev3-cockpit';

function versions(lock) {
    const map = new Map();
    for (const block of lock.split('[[package]]').slice(1)) {
        const name = /^name = "([^"]+)"/m.exec(block)?.[1];
        const version = /^version = "([^"]+)"/m.exec(block)?.[1];
        const local = !/^source = /m.test(block);
        if (name && version && !local) {
            if (!map.has(name)) {
                map.set(name, new Set());
            }
            map.get(name).add(version);
        }
    }
    return map;
}

async function cockpitLock(given) {
    if (given !== undefined) {
        return fs.readFileSync(given, 'utf8');
    }
    const rev = fs.readFileSync(path.join(ROOT, 'backend', 'cockpit.rev'), 'utf8').trim();
    if (!/^[0-9a-f]{40}$/.test(rev)) {
        throw new Error(`backend/cockpit.rev must hold one full commit hash, not "${rev}".`);
    }
    const url = `https://raw.githubusercontent.com/${REPO}/${rev}/daemon/Cargo.lock`;
    const response = await fetch(url);
    if (!response.ok) {
        throw new Error(`Reading ${url} failed: HTTP ${response.status}.`);
    }
    return response.text();
}

async function main() {
    const ours = versions(fs.readFileSync(path.join(ROOT, 'backend', 'Cargo.lock'), 'utf8'));
    const theirs = versions(await cockpitLock(process.argv[2]));
    const differ = [];
    let shared = 0;
    for (const [name, mine] of ours) {
        const cockpit = theirs.get(name);
        if (cockpit === undefined) {
            continue;
        }
        shared++;
        const extra = [...mine].filter(version => !cockpit.has(version));
        if (extra.length > 0) {
            differ.push(`${name}: ev3-c ${[...mine].join(', ')}, EVƎ Cockpit ${[...cockpit].join(', ')}`);
        }
    }
    if (differ.length > 0) {
        console.error(`ev3-c builds ${differ.length} crate(s) in a version EVƎ Cockpit does not:\n  ${differ.join('\n  ')}`);
        console.error('Take the cockpit\'s versions (cargo update -p <crate> --precise <version>), or raise them in the cockpit first.');
        return 1;
    }
    console.log(`${shared} crate(s) both projects use resolve to the same versions.`);
    return 0;
}

main().then(code => { process.exitCode = code; }, error => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
});
