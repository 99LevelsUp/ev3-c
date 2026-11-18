/**
 * Minimal vscode module stub for unit tests running outside the extension host.
 * It grows only as far as host modules under test need it.
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

module.exports = {
    Disposable,
};
