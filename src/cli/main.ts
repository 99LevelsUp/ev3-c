import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { connectCockpit } from '../cockpit/cockpitLink';
import { serveStdio } from '../mcp/server';
import { cliCore, CliDeps, EXIT, runCli } from './cli';

// The `ev3-c` program: what the cockpit's `ev3 c …` starts. It wires runCli to this
// process: its arguments, folders, environment, output and Ctrl+C.

/** EVƎ C's install folder: the nearest folder above this file with ev3lib's headers. */
export function findPackageRoot(from: string): string {
    for (let dir = from; ; dir = path.dirname(dir)) {
        if (fs.existsSync(path.join(dir, 'ev3lib', 'include')) && fs.existsSync(path.join(dir, 'package.json'))) {
            return dir;
        }
        if (path.dirname(dir) === dir) {
            throw new Error(`EVƎ C's files (ev3lib/include) were not found above ${from}.`);
        }
    }
}

/** Where the downloaded compiler is kept for this user, by OS convention. */
export function toolchainCacheDir(env: Readonly<Record<string, string | undefined>>, platform = process.platform, home = os.homedir()): string {
    if (platform === 'win32') {
        return path.join(env.LOCALAPPDATA ?? path.join(home, 'AppData', 'Local'), 'ev3-c', 'toolchain');
    }
    if (platform === 'darwin') {
        return path.join(home, 'Library', 'Caches', 'ev3-c', 'toolchain');
    }
    return path.join(env.XDG_CACHE_HOME ?? path.join(home, '.cache'), 'ev3-c', 'toolchain');
}

async function main(): Promise<number> {
    const packageRoot = findPackageRoot(__dirname);
    const version = String((JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8')) as { version?: unknown }).version ?? '0.0.0');
    const argv = process.argv.slice(2);
    const deps: CliDeps = {
        connect: connectCockpit,
        core: cliCore,
        packageRoot,
        toolchainCacheDir: toolchainCacheDir(process.env),
        version,
        cwd: process.cwd(),
        env: process.env,
        onInterrupt: handler => {
            process.on('SIGINT', handler);
            return () => { process.off('SIGINT', handler); };
        },
    };
    if (argv[0] === 'mcp' && argv.length === 1) {
        return serveStdio(deps);
    }
    return runCli(argv, {
        out: text => process.stdout.write(`${text}\n`),
        err: text => process.stderr.write(`${text}\n`),
    }, deps);
}

if (require.main === module) {
    main().then(code => {
        process.exitCode = code;
    }, (error: unknown) => {
        process.stderr.write(`ev3 c: ${error instanceof Error ? error.message : String(error)}\n`);
        process.exitCode = EXIT.failed;
    });
}
