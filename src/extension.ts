import * as path from 'node:path';
import * as vscode from 'vscode';
import { connectCockpit, CockpitClient } from './cockpit/nativeCockpit';
import { BrickAccess, defaultCore, HostEnvironment, registerBrickCommands, registerProjectCommands } from './host/commands';
import { CockpitSession } from './host/cockpitSession';
import { askPerson } from './host/questions';
import { brickSetting, SETTING_BRICK } from './host/settings';
import { ProgramStatusBar } from './host/statusBar';

// Build and Check Toolchain work on their own. The brick commands reach the bricks through
// EVƎ Cockpit's daemon; EVƎ C connects to it in the background at activation and again on
// each brick command until a connection holds.

/** How EVƎ C names itself to the cockpit: other clients read it in `brick_busy` and `stoppedBy`. */
export const CLIENT_NAME = 'EVƎ C (VS Code)';

/** How EVƎ C reaches the cockpit's daemon; tests replace it. */
export const cockpitConnector = { connect: connectCockpit };

export function activate(context: vscode.ExtensionContext): void {
    const output = vscode.window.createOutputChannel('EVƎ C', { log: true });
    context.subscriptions.push(output);
    const env: HostEnvironment = {
        output,
        packageRoot: context.extensionPath,
        toolchainCacheDir: path.join(context.globalStorageUri.fsPath, 'toolchain'),
        core: defaultCore,
    };
    context.subscriptions.push(...registerProjectCommands(env));

    const version = String((context.extension?.packageJSON as { version?: unknown } | undefined)?.version ?? '0.0.0');
    const session = new CockpitSession(async (): Promise<CockpitClient> => {
        const client = await cockpitConnector.connect({ name: CLIENT_NAME, version });
        client.onQuestion((question, { signal }) => askPerson(question, signal));
        client.onNotice(text => {
            output.warn(`EVƎ Cockpit: ${text}`);
            void vscode.window.showWarningMessage(text);
        });
        output.info(`Connected to EVƎ Cockpit's daemon ${client.daemon.version ?? ''}`.trimEnd());
        return client;
    });
    const bricks: BrickAccess = {
        current: () => session.brick(brickSetting()),
        launches: new Map(),
    };
    const statusBar = new ProgramStatusBar(session);
    context.subscriptions.push(
        session,
        statusBar,
        ...registerBrickCommands(env, bricks),
        vscode.workspace.onDidChangeConfiguration(event => {
            if (event.affectsConfiguration(SETTING_BRICK)) {
                statusBar.update();
            }
        }),
    );
    // Connect now so the status bar shows the brick; a failure is told once and tried again by the next brick command.
    session.ready().catch((error: unknown) => {
        output.warn(`EVƎ C could not connect to EVƎ Cockpit yet: ${error instanceof Error ? error.message : String(error)}`);
    });
}

export function deactivate(): void {
    // Everything is released through the extension context's subscriptions.
}
