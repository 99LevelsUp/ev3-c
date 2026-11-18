'use strict';
// Builds the brick runtime that ships inside the extension into out/runtime/:
// libev3.a, and the supervisor once supervisor/src has sources. Zig comes from
// the core toolchain manager and the target flags from its manifest, the same
// ones the student's build uses. A small program linked against libev3.a must
// then pass the core ELF check, which proves the library links into an
// executable the brick can run.
const fs = require('fs');
const path = require('path');
const { root, loadCore, ensureZig, sourcesIn, buildEv3lib } = require('./ev3lib-build.cjs');

const outDir = 'out/runtime';
const workDir = 'build/runtime';

// Uses ev3lib's whole surface and malloc, which fails to link for ARMv5 with
// toolchains whose allocator needs atomic instructions the brick lacks.
const checkProgram = `#include <stdio.h>
#include <stdlib.h>
#include "ev3.h"

int main(void)
{
    char *line = malloc(32);
    if (line == NULL) {
        return 1;
    }
    snprintf(line, 32, "run folder %s", ev3_core_run_dir() ? ev3_core_run_dir() : "-");
    puts(line);
    free(line);
    ev3_button_take_back();
    ev3_motor_set_power(EV3_PORT_A | EV3_PORT_B, 50);
    while (!ev3_button_back_pressed()) {
        ev3_core_sleep_ms(10);
    }
    ev3_motor_stop(EV3_PORT_ALL, 1);
    ev3_button_release_back();
    ev3_core_exit();
    return 0;
}
`;

async function checkedLink(core, compiler, request, label) {
    const result = await core.compiler.compile({ compilerPath: compiler, cwd: root, ...request });
    if (result.diagnostics.trim() !== '') {
        process.stdout.write(result.diagnostics);
    }
    if (!result.ok) {
        throw new Error(`${label}: linking ${request.output} failed with exit code ${result.exitCode}.`);
    }
    const check = await core.elf.checkElf(path.join(root, request.output));
    if (!check.ok) {
        throw new Error(`${label}: ${request.output} fails the ELF check:\n  ${check.problems.join('\n  ')}`);
    }
    console.log(`${label}: ${request.output} passes the ELF check (static ARMv5TE soft-float)`);
}

async function main() {
    const core = loadCore();
    const compiler = await ensureZig(core);
    // Linker flags stay out of the object builds: Zig applies -Wl,--gc-sections
    // to a single object compiled with -c and keeps only what the constructor
    // reaches, which leaves the library without its public functions.
    const objectFlags = core.toolchain.manifest.targetFlags.filter(flag => !flag.startsWith('-Wl,'));

    fs.rmSync(path.join(root, outDir), { recursive: true, force: true });
    fs.rmSync(path.join(root, workDir), { recursive: true, force: true });
    fs.mkdirSync(path.join(root, outDir), { recursive: true });

    const archive = `${outDir}/libev3.a`;
    buildEv3lib(compiler, objectFlags, `${workDir}/obj`, archive);

    const supervisorSources = sourcesIn('supervisor/src');
    if (supervisorSources.length === 0) {
        console.log('supervisor: skipped, supervisor/src has no C sources yet');
    } else {
        await checkedLink(core, compiler, {
            sources: supervisorSources,
            includeDirs: ['supervisor/src'],
            libraries: [],
            output: `${outDir}/supervisor`,
        }, 'supervisor');
    }

    fs.mkdirSync(path.join(root, workDir, 'check'), { recursive: true });
    fs.writeFileSync(path.join(root, workDir, 'check', 'check.c'), checkProgram);
    await checkedLink(core, compiler, {
        sources: [`${workDir}/check/check.c`],
        includeDirs: ['ev3lib/include'],
        libraries: [archive],
        output: `${workDir}/check/check`,
    }, 'libev3.a');

    console.log(`runtime ready in ${outDir}/: ${fs.readdirSync(path.join(root, outDir)).join(', ')}`);
}

main().catch(error => {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
});
