import * as vscode from 'vscode';
import { Subscription } from '../core';
import { isBrickOnline } from '../cockpit/cockpitLink';
import { CockpitSession, Connected, resolveSetting } from './cockpitSession';
import { COMMAND_IDS } from './commands';
import { brickSetting } from './settings';

// A status bar item that shows the user-slot state cockpit reports for the brick the
// "ev3-c.brick" setting names. State events are changes only, so the state is asked once
// after binding to a brick and again after every reconnect. Until a brick command has
// connected EVƎ C to the cockpit, the item says so.

type Shown = 'running' | 'idle' | 'disconnected' | 'no-cockpit';

export class ProgramStatusBar implements vscode.Disposable {
    private readonly item: vscode.StatusBarItem;
    private readonly sessionWatch: { dispose(): void };
    private bricksWatch: { dispose(): void } | undefined;
    private bound: { brickId: string; events: Promise<Subscription | undefined> } | undefined;

    constructor(private readonly session: CockpitSession) {
        this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
        this.item.name = 'EVƎ C program';
        this.sessionWatch = session.onChanged(connected => this.attach(connected));
        this.attach(session.current());
        this.item.show();
    }

    /** Follows the brick the setting names now; called when bricks or the setting change. */
    update(): void {
        const connected = this.session.current();
        if (connected === undefined) {
            this.unbind();
            this.show('no-cockpit');
            return;
        }
        let brickId: string;
        try {
            brickId = resolveSetting(connected, brickSetting());
        } catch {
            this.unbind();
            this.show('disconnected');
            return;
        }
        const online = isBrickOnline(connected.client.bricks(), brickId);
        if (this.bound?.brickId === brickId) {
            // While the brick is away the port subscribes again by itself and says so.
            if (!online) {
                this.show('disconnected');
            }
            return;
        }
        this.unbind();
        if (!online) {
            this.show('disconnected');
            return;
        }
        const { port } = connected.ports.get(brickId);
        const events = port.onBrickEvent(event => {
            if (event.kind === 'program') {
                this.show(event.state.running ? 'running' : 'idle');
            } else {
                this.refresh(connected, brickId);
            }
        }).catch(() => {
            // Not followable yet; the next change of the brick list tries again.
            if (this.bound?.brickId === brickId) {
                this.bound = undefined;
                this.show('disconnected');
            }
            return undefined;
        });
        this.bound = { brickId, events };
        this.refresh(connected, brickId);
    }

    dispose(): void {
        this.unbind();
        this.sessionWatch.dispose();
        this.bricksWatch?.dispose();
        this.item.dispose();
    }

    private attach(connected: Connected | undefined): void {
        this.bricksWatch?.dispose();
        if (connected !== undefined) {
            // The bar follows the bricks and, for "active", the brick the cockpit's window shows.
            const watches = [connected.client.onBricksChanged(() => this.update()), connected.shown.onChanged(() => this.update())];
            this.bricksWatch = { dispose: () => watches.forEach(w => w.dispose()) };
        } else {
            this.bricksWatch = undefined;
        }
        this.unbind();
        this.update();
    }

    private refresh(connected: Connected, brickId: string): void {
        connected.ports.get(brickId).port.programState().then(
            state => {
                if (this.bound?.brickId === brickId) {
                    this.show(state.running ? 'running' : 'idle');
                }
            },
            () => this.show('disconnected'),
        );
    }

    private unbind(): void {
        void this.bound?.events.then(events => events?.dispose());
        this.bound = undefined;
    }

    private show(shown: Shown): void {
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
            this.item.tooltip = 'No brick is connected, or several are. Connect one in EVƎ Cockpit, or set "ev3-c.brick".';
            this.item.command = undefined;
            break;
        case 'no-cockpit':
            this.item.text = '$(debug-disconnect) EV3 C: not connected';
            this.item.tooltip = 'EVƎ C connects to EVƎ Cockpit on the first brick command.';
            this.item.command = undefined;
            break;
        }
    }
}
