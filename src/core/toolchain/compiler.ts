import { spawn } from 'node:child_process';
import { manifest } from './toolchainManager';

// Cross-compiles C sources for the brick with `zig cc`. The target flags come
// from manifest.json, the single place that defines what a brick binary is.

export interface CompileRequest {
    /** Compiler executable, as returned by ensureCompiler. */
    compilerPath: string;
    sources: string[];
    includeDirs: string[];
    /** Static libraries (for example libev3.a), passed to the linker by path. */
    libraries: string[];
    output: string;
    cwd?: string;
}

export interface CompileResult {
    ok: boolean;
    elfPath: string;
    exitCode: number | null;
    /** Everything the compiler wrote to stdout and stderr, in order. */
    diagnostics: string;
}

function compileArgs(request: CompileRequest): string[] {
    return [
        'cc',
        ...manifest.targetFlags,
        ...request.includeDirs.map(dir => `-I${dir}`),
        ...request.sources,
        ...request.libraries,
        '-o', request.output,
    ];
}

export function compile(request: CompileRequest): Promise<CompileResult> {
    return new Promise((resolve, reject) => {
        const child = spawn(request.compilerPath, compileArgs(request), { cwd: request.cwd, windowsHide: true });
        let diagnostics = '';
        child.stdout.on('data', (chunk: Buffer) => { diagnostics += chunk.toString(); });
        child.stderr.on('data', (chunk: Buffer) => { diagnostics += chunk.toString(); });
        child.on('error', reject);
        child.on('close', exitCode => {
            resolve({ ok: exitCode === 0, elfPath: request.output, exitCode, diagnostics });
        });
    });
}
