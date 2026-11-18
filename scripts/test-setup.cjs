/**
 * Preload script for unit tests. Resolves require('vscode') to a stub so that
 * host modules load outside the extension host.
 */
'use strict';

const Module = require('module');
const path = require('path');

const stubPath = path.join(__dirname, '..', 'src', 'test', 'vscodeStub.cjs');

const originalResolve = Module._resolveFilename;
Module._resolveFilename = function (request, parent, isMain, options) {
    if (request === 'vscode') {
        return stubPath;
    }
    return originalResolve.call(this, request, parent, isMain, options);
};
