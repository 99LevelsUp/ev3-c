import * as vscode from 'vscode';
import { Ev3c, Ev3cEvent } from '../backend/ev3c';
import { COMMAND_IDS } from './commands';

// A status bar item that shows the program state of the brick the "ev3-c.brick" setting names.
// It runs `ev3-c watch` for as long as it lives. That command only follows the brick and never
// connects or holds one. When `watch` ends on its own (no cockpit, or the cockpit went away), the
// item says so and starts it again after a pause. A change of the setting starts it anew.

type Shown = 'running' | 'idle' | 'disconnected' | 'no-cockpit';

/** How long to wait before following again after `watch` ended on its own. */
export const RETRY_MS = 10_000;

export class ProgramStatusBar implements vscode.Disposable {
    private readonly item: vscode.StatusBarItem;
    private watch: { stop(): void } | undefined;
    private retry: NodeJS.Timeout | undefined;
    private disposed = false;

    constructor(private readonly ev3c: Ev3c, private readonly output: vscode.LogOutputChannel) {
        this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
        this.item.name = 'EVƎ C program';
        this.show('no-cockpit');
        this.item.show();
        this.start();
    }

    /** Follows anew, for a changed "ev3-c.brick" setting. */
    restart(): void {
        this.stopWatching();
        this.start();
    }

    dispose(): void {
        this.disposed = true;
        this.stopWatching();
        this.item.dispose();
    }

    private start(): void {
        if (this.disposed) {
            return;
        }
        const watch = this.ev3c.follow(['watch'], event => this.heard(event));
        this.watch = watch;
        const ended = (why: string | undefined) => {
            if (this.watch !== watch) {
                return; // Stopped on purpose.
            }
            this.watch = undefined;
            this.show('no-cockpit', why);
            this.retry = setTimeout(() => {
                this.retry = undefined;
                this.start();
            }, RETRY_MS);
            this.retry.unref?.();
        };
        watch.ended.then(
            terminal => {
                const error = terminal.error as { message?: unknown } | undefined;
                ended(typeof error?.message === 'string' ? error.message : undefined);
            },
            (error: unknown) => ended(error instanceof Error ? error.message : String(error)),
        );
    }

    private stopWatching(): void {
        if (this.retry !== undefined) {
            clearTimeout(this.retry);
            this.retry = undefined;
        }
        const watch = this.watch;
        this.watch = undefined;
        watch?.stop();
    }

    private heard(event: Ev3cEvent): void {
        if (event.event === 'program') {
            this.show(event.running === true ? 'running' : 'idle');
        } else if (event.event === 'noBrick') {
            const error = event.error as { message?: unknown } | undefined;
            this.show('disconnected', typeof error?.message === 'string' ? error.message : undefined);
        } else if (event.event === 'notice') {
            this.output.warn(`EVƎ Cockpit: ${String(event.text)}`);
        }
    }

    private show(shown: Shown, why?: string): void {
        switch (shown) {
        case 'running':
            this.item.text = '$(play) EV3 C: running';
            this.item.tooltip = 'A program is running on the brick. Click to stop it.';
            this.item.command = COMMAND_IDS.stop;
            break;
        case 'idle':
            this.item.text = '$(circle-outline) EV3 C: idle';
            this.item.tooltip = 'No program is running on the brick.';
            this.item.command = undefined;
            break;
        case 'disconnected':
            this.item.text = '$(debug-disconnect) EV3 C: no brick';
            this.item.tooltip = `${why ?? 'No brick is connected.'} Connect one in EVƎ Cockpit, or set "ev3-c.brick".`;
            this.item.command = undefined;
            break;
        case 'no-cockpit':
            this.item.text = '$(debug-disconnect) EV3 C: not connected';
            this.item.tooltip = why === undefined ? 'EVƎ C is connecting to EVƎ Cockpit.' : `EVƎ C cannot reach EVƎ Cockpit: ${why}`;
            this.item.command = undefined;
            break;
        }
    }
}
