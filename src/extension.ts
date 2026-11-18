import * as path from 'node:path';
import * as vscode from 'vscode';
import { BrickAccess, defaultCore, HostEnvironment, registerBrickCommands, registerProjectCommands } from './host/commands';
import { BrickPorts, connectCockpit, resolveBrickId } from './host/cockpitAdapter';
import { brickSetting, SETTING_BRICK } from './host/settings';
import { ProgramStatusBar } from './host/statusBar';

// Build and Check Toolchain work on their own. The brick commands need EVƎ Cockpit's API
// version 1; without it EVƎ C says so once and registers none of them.

export async function activate(context: vscode.ExtensionContext): Promise<void> {
    const output = vscode.window.createOutputChannel('EVƎ C', { log: true });
    context.subscriptions.push(output);
    const env: HostEnvironment = {
        output,
        packageRoot: context.extensionPath,
        toolchainCacheDir: path.join(context.globalStorageUri.fsPath, 'toolchain'),
        core: defaultCore,
    };
    context.subscriptions.push(...registerProjectCommands(env));

    const cockpit = await connectCockpit(id => vscode.extensions.getExtension(id));
    if (!cockpit.ok) {
        output.error(cockpit.problem);
        void vscode.window.showErrorMessage(cockpit.problem);
        return;
    }
    const { consumer } = cockpit;
    const ports = new BrickPorts(consumer);
    const bricks: BrickAccess = {
        current: () => ports.get(resolveBrickId(consumer, brickSetting())),
        launches: new Map(),
    };
    const statusBar = new ProgramStatusBar(consumer, ports);
    context.subscriptions.push(
        consumer,
        ports,
        statusBar,
        ...registerBrickCommands(env, bricks),
        vscode.workspace.onDidChangeConfiguration(event => {
            if (event.affectsConfiguration(SETTING_BRICK)) {
                statusBar.update();
            }
        }),
    );
}

export function deactivate(): void {
    // Everything is released through the extension context's subscriptions.
}
