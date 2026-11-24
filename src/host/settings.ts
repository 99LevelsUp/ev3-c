import * as vscode from 'vscode';

// EVƎ C's settings. Their defaults live in package.json only.

export const SETTING_BRICK = 'ev3-c.brick';
export const SETTING_TOOLCHAIN_PATH = 'ev3-c.toolchain.path';

/** The brick EVƎ C works with: `active`, or a brick id as EVƎ Cockpit lists it. */
export function brickSetting(): string {
    const value = vscode.workspace.getConfiguration('ev3-c').get<string>('brick')?.trim();
    // An empty value names no brick, which is what `active` means.
    return value === undefined || value === '' ? 'active' : value;
}

/** The compiler path set by the author, or undefined to use the pinned, downloaded one. */
export function toolchainPath(): string | undefined {
    const value = vscode.workspace.getConfiguration('ev3-c').get<string>('toolchain.path')?.trim();
    return value === undefined || value === '' ? undefined : value;
}
