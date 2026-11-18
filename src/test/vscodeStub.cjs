/**
 * Minimal vscode module stub for unit tests running outside the extension host.
 * It grows only as far as host modules under test need it. Tests read what the host did
 * from `__state` (messages shown, commands registered, output lines) and set the
 * workspace, settings, and installed extensions there; `__reset()` starts afresh.
 */
'use strict';

class Disposable {
    constructor(callOnDispose) {
        this._callOnDispose = callOnDispose;
    }

    dispose() {
        if (this._callOnDispose) {
            this._callOnDispose();
            this._callOnDispose = undefined;
        }
    }
}

function freshState() {
    return {
        /** Shown messages as { level, text }. */
        messages: [],
        /** Registered command handlers by ID. */
        commands: new Map(),
        /** Lines written to any output channel. */
        output: [],
        statusBarItems: [],
        /** Setting values by full key, e.g. "ev3-c.brick". */
        config: new Map(),
        configListeners: [],
        workspaceFolders: undefined,
        isTrusted: true,
        /** Extensions by ID, as getExtension returns them. */
        extensions: new Map(),
        /** Answers for showQuickPick: a function (items) => item, or undefined to dismiss. */
        quickPick: undefined,
        /** Answer for showInputBox. */
        inputBox: undefined,
        /** Picks the item a message resolves to: (text, items) => item; dismissed when unset. */
        messageAnswer: undefined,
    };
}

let state = freshState();

function message(level) {
    return (text, ...rest) => {
        state.messages.push({ level, text });
        return Promise.resolve(state.messageAnswer?.(text, rest));
    };
}

const window = {
    showErrorMessage: message('error'),
    showWarningMessage: message('warning'),
    showInformationMessage: message('info'),
    showQuickPick: async (items) => {
        const resolved = await items;
        return state.quickPick ? state.quickPick(resolved) : undefined;
    },
    showInputBox: async () => state.inputBox,
    createOutputChannel: (name) => ({
        name,
        appendLine: (line) => { state.output.push(line); },
        append: (text) => { state.output.push(text); },
        show: () => undefined,
        dispose: () => undefined,
    }),
    createStatusBarItem: () => {
        const item = { text: '', tooltip: undefined, command: undefined, visible: false };
        item.show = () => { item.visible = true; };
        item.hide = () => { item.visible = false; };
        item.dispose = () => undefined;
        state.statusBarItems.push(item);
        return item;
    },
    withProgress: (_options, task) => task({ report: () => undefined }, { isCancellationRequested: false }),
};

const commands = {
    registerCommand: (id, handler) => {
        if (state.commands.has(id)) {
            throw new Error(`command '${id}' already exists`);
        }
        state.commands.set(id, handler);
        return new Disposable(() => state.commands.delete(id));
    },
    executeCommand: async (id, ...args) => {
        const handler = state.commands.get(id);
        if (!handler) {
            throw new Error(`command '${id}' not found`);
        }
        return handler(...args);
    },
};

const workspace = {
    get isTrusted() {
        return state.isTrusted;
    },
    get workspaceFolders() {
        return state.workspaceFolders;
    },
    getConfiguration: (section) => ({
        get: (key) => state.config.get(section ? `${section}.${key}` : key),
    }),
    onDidChangeConfiguration: (listener) => {
        state.configListeners.push(listener);
        return new Disposable(() => undefined);
    },
};

const extensions = {
    getExtension: (id) => state.extensions.get(id),
};

module.exports = {
    Disposable,
    window,
    commands,
    workspace,
    extensions,
    StatusBarAlignment: { Left: 1, Right: 2 },
    ProgressLocation: { SourceControl: 1, Window: 10, Notification: 15 },
    get __state() {
        return state;
    },
    __reset() {
        state = freshState();
    },
};
