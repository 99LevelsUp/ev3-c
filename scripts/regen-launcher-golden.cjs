'use strict';
/**
 * Regenerates the golden launcher bytes from backend/crates/core/fixtures/launcher/launcher.lms
 * with LEGO's lmsasm assembler. Development-time only: it needs Java, and normal test runs
 * read the committed launcher.rbf.hex instead.
 *
 * Usage: node scripts/regen-launcher-golden.cjs <lmsasm-folder>
 *
 * <lmsasm-folder> is the firmware developer edition's lmssrc/adk/lmsasm folder (it holds
 * assembler.jar next to bytecodes.c and bytecodes.h). The folder is copied to a temporary
 * directory, so the assembler's outputs never land in the source tree.
 */
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawnSync } = require('child_process');

const fixtures = path.join(__dirname, '..', 'backend', 'crates', 'core', 'fixtures', 'launcher');
const source = path.join(fixtures, 'launcher.lms');
const golden = path.join(fixtures, 'launcher.rbf.hex');

function fail(message) {
    process.stderr.write(`regen-launcher-golden: ${message}\n`);
    process.exit(1);
}

const lmsasm = process.argv[2];
if (!lmsasm) {
    fail('usage: node scripts/regen-launcher-golden.cjs <lmsasm-folder>');
}
if (!fs.existsSync(path.join(lmsasm, 'assembler.jar'))) {
    fail(`no assembler.jar in ${lmsasm}`);
}

const work = fs.mkdtempSync(path.join(os.tmpdir(), 'ev3c-lmsasm-'));
try {
    fs.cpSync(lmsasm, work, { recursive: true });
    fs.copyFileSync(source, path.join(work, 'launcher.lms'));

    // lmsasm takes the source path without its .lms extension and writes <path>.rbf.
    const run = spawnSync('java', ['-Djava.awt.headless=true', '-jar', 'assembler.jar', path.join(work, 'launcher')], {
        cwd: work,
        encoding: 'utf8',
    });
    if (run.error) {
        fail(`cannot run java: ${run.error.message}`);
    }
    process.stdout.write(run.stdout);
    process.stderr.write(run.stderr);

    const rbf = path.join(work, 'launcher.rbf');
    if (run.status !== 0 || !fs.existsSync(rbf)) {
        fail(`lmsasm did not produce launcher.rbf (exit ${run.status})`);
    }

    const bytes = fs.readFileSync(rbf);
    const lines = [];
    for (let i = 0; i < bytes.length; i += 16) {
        lines.push(Array.from(bytes.subarray(i, i + 16), b => b.toString(16).padStart(2, '0')).join(' '));
    }
    fs.writeFileSync(golden, lines.join('\n') + '\n');
    process.stdout.write(`wrote ${path.relative(process.cwd(), golden)} (${bytes.length} bytes)\n`);
} finally {
    fs.rmSync(work, { recursive: true, force: true });
}
