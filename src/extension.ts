import * as vscode from 'vscode';
import { CLIENT_NAME, Ev3c, findProgram, Spawn } from './backend/ev3c';
import { HostEnvironment, launchesVariable, registerCommands } from './host/commands';
import { brickSetting, SETTING_BRICK, SETTING_TOOLCHAIN_PATH, toolchainPath } from './host/settings';
import { ProgramStatusBar } from './host/statusBar';

// The extension is EVƎ C's UI: commands, the status bar, notifications and settings. All work
// is done by the Rust program `ev3-c`, which it runs once per command (and once, for as long
// as the window lasts, to follow the brick for the status bar). `ev3-c` reaches the bricks
// through EVƎ Cockpit's daemon, under the name "EVƎ C (VS Code)".

/** How the extension starts processes; tests replace it. */
export const processes: { spawn?: Spawn } = {};

export function activate(context: vscode.ExtensionContext): void {
    const output = vscode.window.createOutputChannel('EVƎ C', { log: true });
    context.subscriptions.push(output);
    const launches = new Map<string, string>();
    const ev3c = new Ev3c(
        () => findProgram(context.extensionPath),
        () => {
            const toolchain = toolchainPath();
            return {
                EV3C_PACKAGE_ROOT: context.extensionPath,
                EV3C_CLIENT_NAME: CLIENT_NAME,
                EV3C_BRICK: brickSetting(),
                EV3C_LAUNCHES: launchesVariable(launches),
                ...(toolchain !== undefined ? { EV3C_TOOLCHAIN: toolchain, EV3C_TOOLCHAIN_SETTING: SETTING_TOOLCHAIN_PATH } : {}),
            };
        },
        processes.spawn,
    );
    const env: HostEnvironment = { output, ev3c, launches };
    const statusBar = new ProgramStatusBar(ev3c, output);
    context.subscriptions.push(
        statusBar,
        ...registerCommands(env),
        vscode.workspace.onDidChangeConfiguration(event => {
            if (event.affectsConfiguration(SETTING_BRICK)) {
                statusBar.restart();
            }
        }),
    );
}

export function deactivate(): void {
    // Everything is released through the extension context's subscriptions.
}
