import * as vscode from 'vscode';
import { Subscription } from '../core';
import { BrickPorts, isBrickOnline, resolveBrickId } from './cockpitAdapter';
import { CockpitConsumer, Subscription as CockpitSubscription } from './cockpitTypes';
import { COMMAND_IDS } from './commands';
import { brickSetting } from './settings';

// A status bar item that shows the user-slot state cockpit reports for the brick the
// "ev3-c.brick" setting names. State events are changes only, so the state is asked once
// after binding to a brick and again after every reconnect.

type Shown = 'running' | 'idle' | 'disconnected';

export class ProgramStatusBar implements vscode.Disposable {
    private readonly item: vscode.StatusBarItem;
    private readonly watches: CockpitSubscription[];
    private bound: { brickId: string; events: Subscription } | undefined;

    constructor(private readonly consumer: CockpitConsumer, private readonly ports: BrickPorts) {
        this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left);
        this.item.name = 'EVƎ C program';
        this.watches = [
            consumer.onBricksChanged(() => this.update()),
            consumer.onActiveBrickChanged(() => this.update()),
        ];
        this.update();
        this.item.show();
    }

    /** Follows the brick the setting names now; called when bricks or the setting change. */
    update(): void {
        let brickId: string;
        try {
            brickId = resolveBrickId(this.consumer, brickSetting());
        } catch {
            this.unbind();
            this.show('disconnected');
            return;
        }
        const signedIn = isBrickOnline(this.consumer.listBricks(), brickId);
        if (this.bound?.brickId === brickId) {
            // While the brick is away the port subscribes again by itself and says so.
            if (!signedIn) {
                this.show('disconnected');
            }
            return;
        }
        this.unbind();
        if (!signedIn) {
            this.show('disconnected');
            return;
        }
        const { port } = this.ports.get(brickId);
        try {
            const events = port.onBrickEvent(event => {
                if (event.kind === 'program') {
                    this.show(event.running ? 'running' : 'idle');
                } else {
                    this.refresh(brickId);
                }
            });
            this.bound = { brickId, events };
        } catch {
            // Not followable yet; the next change of the brick list tries again.
            this.show('disconnected');
            return;
        }
        this.refresh(brickId);
    }

    dispose(): void {
        this.unbind();
        for (const watch of this.watches) {
            watch.dispose();
        }
        this.item.dispose();
    }

    private refresh(brickId: string): void {
        this.ports.get(brickId).port.isProgramRunning().then(
            state => {
                if (this.bound?.brickId === brickId) {
                    this.show(state.running ? 'running' : 'idle');
                }
            },
            () => this.show('disconnected'),
        );
    }

    private unbind(): void {
        this.bound?.events.dispose();
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
            this.item.tooltip = 'No brick is connected. Connect one in EVƎ Cockpit, or check the "ev3-c.brick" setting.';
            this.item.command = undefined;
            break;
        }
    }
}
