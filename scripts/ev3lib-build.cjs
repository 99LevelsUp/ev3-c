'use strict';
// Shared by build-runtime.cjs and run-native-tests.cjs: loads the compiled
// toolchain core, obtains Zig through it, and builds ev3lib into a static
// library. Paths passed to Zig are relative to the repository root, so the
// objects do not record where the repository happens to live.
const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const root = path.join(__dirname, '..');

function loadCore() {
    const dir = path.join(root, 'out', 'core', 'toolchain');
    if (!fs.existsSync(path.join(dir, 'toolchainManager.js'))) {
        throw new Error('The compiled core is missing in out/core (run "npm run compile" first).');
    }
    return {
        toolchain: require(path.join(dir, 'toolchainManager.js')),
        compiler: require(path.join(dir, 'compiler.js')),
        elf: require(path.join(dir, 'elfCheck.js')),
    };
}

async function ensureZig(core) {
    const info = await core.toolchain.ensureCompiler({ cacheDir: path.join(root, 'toolchain', 'cache') });
    console.log(`zig ${info.version ?? '(override)'} from ${info.origin}: ${info.path}`);
    return info.path;
}

/** C sources under `dir` (relative to the repository root), sorted, with forward slashes. */
function sourcesIn(dir) {
    const found = [];
    const walk = relative => {
        const absolute = path.join(root, relative);
        if (!fs.existsSync(absolute)) {
            return;
        }
        for (const entry of fs.readdirSync(absolute, { withFileTypes: true })) {
            const child = `${relative}/${entry.name}`;
            if (entry.isDirectory()) {
                walk(child);
            } else if (entry.name.endsWith('.c')) {
                found.push(child);
            }
        }
    };
    walk(dir);
    return found.sort();
}

function zig(compiler, args) {
    const result = spawnSync(compiler, args, { cwd: root, stdio: 'inherit', windowsHide: true });
    if (result.error) {
        throw result.error;
    }
    if (result.status !== 0) {
        throw new Error(`zig ${args.join(' ')} failed with exit code ${result.status}.`);
    }
}

/** Compiles every ev3lib source with `flags` and archives the objects into `archive`. */
function buildEv3lib(compiler, flags, objDir, archive) {
    fs.rmSync(path.join(root, objDir), { recursive: true, force: true });
    fs.mkdirSync(path.join(root, objDir), { recursive: true });
    const objects = sourcesIn('ev3lib/src').map(source => {
        const object = `${objDir}/${source.slice('ev3lib/src/'.length).replace(/\//g, '_').replace(/\.c$/, '.o')}`;
        zig(compiler, ['cc', '-c', ...flags, '-Wall', '-Wextra', '-Werror', '-Iev3lib/include', source, '-o', object]);
        return object;
    });
    fs.rmSync(path.join(root, archive), { force: true });
    fs.mkdirSync(path.dirname(path.join(root, archive)), { recursive: true });
    zig(compiler, ['ar', 'rcs', archive, ...objects]);
    console.log(`built ${archive} from ${objects.length} source file(s)`);
}

module.exports = { root, loadCore, ensureZig, sourcesIn, zig, buildEv3lib };
