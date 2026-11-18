// The launcher: a small bytecode program (.rbf) that keeps the brick's user slot alive while
// the native supervisor runs the student's program, and ends the slot when the program ends.
//
// What it does, in order:
//   1. Build the project folder with FILENAME GET_FOLDERNAME, and every marker path as that
//      folder plus a short dot-name: .tok (slot token), .run (run marker), .bak (Back request).
//      The firmware resolves a bare dot-name against its own working directory, so the folder
//      is always spelled out.
//   2. Remove stale .run and .bak, delete .tok, then create .tok and keep it open for the whole
//      run. The supervisor watches the VM's open descriptor on .tok to notice the slot ending.
//   3. Start the supervisor in the background, once, through SYSTEM (see supervisorArguments).
//   4. Wait up to 6 s for .run, or end the slot. Read the supervisor's PID from .run once and
//      build /proc/<pid>.
//   5. Every 100 ms test .run and /proc/<pid>, and set or clear the Back block when .bak
//      appears or disappears, so a program can take short presses of Back over.
//   6. End when .run is gone or the supervisor's process has disappeared, so a crashed
//      supervisor still ends the slot and the firmware stops the motors.
// No file is closed inside the loop, because every bytecode close runs a global sync(); the
// PID read is the only close after start-up. The launcher changes no file permissions and
// touches nothing outside its own project folder. It uses no project name: everything comes
// from the folder it was started from.
//
// The golden source for these bytes is src/__tests__/fixtures/launcher/launcher.lms; keep the
// two in step (declaration order and sizes decide the local offsets).

import { AssembledImage, FIRMWARE_LIMITS, label, local, op, str, sub, ThreadBuilder } from './bytecode';

/** How many run folders the supervisor keeps before it creates a new one. */
export type RetentionRule =
    | { readonly kind: 'runs'; readonly count: number }
    | { readonly kind: 'megabytes'; readonly limit: number };

export interface LauncherOptions {
    readonly retention: RetentionRule;
    /** Size limit in bytes for each captured output stream (stdout, stderr). */
    readonly outputCapBytes: number;
}

/** File name of the supervisor executable inside the project folder. */
export const SUPERVISOR_FILE = 'sup';

const TICK_MS = 100;
const START_TIMEOUT_MS = 6000;

// The .run marker holds the supervisor's PID as decimal digits without a newline (written to
// a temporary name and renamed into place). The brick's kernel hands out PIDs of at most five
// digits; 15 is the read limit and leaves room.
const PID_READ_BYTES = 15;

// STRINGS ADD writes into a local with an unbounded sprintf, so every buffer must hold the
// longest value it can receive. A program's folder (with its trailing '/') is shorter than
// vmPATHSIZE, so GET_FOLDERNAME never truncates it into FOLDER_SIZE.
const FOLDER_SIZE = FIRMWARE_LIMITS.PATH_SIZE;
const PATH_SIZE = FIRMWARE_LIMITS.FILENAME_SIZE;
const COMMAND_SIZE = 256;

const MAX_ARGUMENT = 0x7fffffff;

function positiveInt32(value: number, what: string): number {
    if (!Number.isInteger(value) || value <= 0 || value > MAX_ARGUMENT) {
        throw new RangeError(`${what} must be a whole number from 1 to ${MAX_ARGUMENT}, got ${value}`);
    }
    return value;
}

/**
 * The supervisor's arguments after the project folder. The full command line is
 *
 *     <folder>/sup <folder> keep=<n> cap=<bytes> &
 *     <folder>/sup <folder> keepmb=<m> cap=<bytes> &
 *
 * argv[1] is the project folder as the VM reports it (absolute when started through cockpit,
 * relative to the VM's working directory when started from the brick menu), argv[2] the
 * retention rule (keep the last n runs, or runs up to m megabytes), argv[3] the cap on each
 * captured output stream in bytes. Every number is a positive decimal that fits a signed
 * 32-bit integer.
 */
export function supervisorArguments(options: LauncherOptions): string {
    const { retention } = options;
    const rule = retention.kind === 'runs'
        ? `keep=${positiveInt32(retention.count, 'run count')}`
        : `keepmb=${positiveInt32(retention.limit, 'megabyte limit')}`;
    return `${rule} cap=${positiveInt32(options.outputCapBytes, 'output cap')}`;
}

/** Assembles the launcher and keeps its layout, for inspection. */
export function assembleLauncher(options: LauncherOptions): AssembledImage {
    const tail = ` ${supervisorArguments(options)} &`;
    const t = new ThreadBuilder();

    t.dataString('Folder', FOLDER_SIZE);
    t.dataString('RunPath', PATH_SIZE);
    t.dataString('BakPath', PATH_SIZE);
    t.dataString('TokPath', PATH_SIZE);
    t.dataString('T1', COMMAND_SIZE);
    t.dataString('T2', COMMAND_SIZE);
    t.dataString('Cmd', COMMAND_SIZE);
    t.dataString('Pid', PID_READ_BYTES + 1);
    t.dataString('Proc', 32);
    t.data16('hTok');
    t.data16('hRun');
    t.data32('Size');
    t.data32('Status');
    t.data32('Timer');
    t.data8('Flag');
    t.data8('BackOn');
    t.data8('Count');

    // Marker paths from the project folder.
    t.emit(op.FILENAME, sub.FILENAME.GET_FOLDERNAME, FOLDER_SIZE, local('Folder'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('Folder'), str('/.run'), local('RunPath'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('Folder'), str('/.bak'), local('BakPath'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('Folder'), str('/.tok'), local('TokPath'));

    // Clear stale markers and hold a fresh slot token.
    t.emit(op.FILE, sub.FILE.REMOVE, local('RunPath'));
    t.emit(op.FILE, sub.FILE.REMOVE, local('BakPath'));
    t.emit(op.FILE, sub.FILE.REMOVE, local('TokPath'));
    t.emit(op.FILE, sub.FILE.OPEN_WRITE, local('TokPath'), local('hTok'));

    // Start the supervisor in the background.
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('Folder'), str(`/${SUPERVISOR_FILE} `), local('T1'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('T1'), local('Folder'), local('T2'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, local('T2'), str(tail), local('Cmd'));
    t.emit(op.SYSTEM, local('Cmd'), local('Status'));

    // Wait for the run marker.
    t.emit(op.MOVE8_8, 0, local('Count'));
    t.label('WaitStart');
    t.emit(op.TIMER_WAIT, TICK_MS, local('Timer'));
    t.emit(op.TIMER_READY, local('Timer'));
    t.emit(op.FILENAME, sub.FILENAME.EXIST, local('RunPath'), local('Flag'));
    t.emit(op.JR_TRUE, local('Flag'), label('Started'));
    t.emit(op.ADD8, local('Count'), 1, local('Count'));
    t.emit(op.JR_LT8, local('Count'), START_TIMEOUT_MS / TICK_MS, label('WaitStart'));
    t.emit(op.JR, label('Done'));

    // Read the supervisor's PID once.
    t.label('Started');
    t.emit(op.FILE, sub.FILE.OPEN_READ, local('RunPath'), local('hRun'), local('Size'));
    t.emit(op.FILE, sub.FILE.READ_TEXT, local('hRun'), 0, PID_READ_BYTES, local('Pid'));
    t.emit(op.FILE, sub.FILE.CLOSE, local('hRun'));
    t.emit(op.STRINGS, sub.STRINGS.ADD, str('/proc/'), local('Pid'), local('Proc'));
    t.emit(op.MOVE8_8, 0, local('BackOn'));

    // Poll until the run marker or the supervisor is gone.
    t.label('Poll');
    t.emit(op.TIMER_WAIT, TICK_MS, local('Timer'));
    t.emit(op.TIMER_READY, local('Timer'));
    t.emit(op.FILENAME, sub.FILENAME.EXIST, local('RunPath'), local('Flag'));
    t.emit(op.JR_FALSE, local('Flag'), label('Done'));
    t.emit(op.FILENAME, sub.FILENAME.EXIST, local('Proc'), local('Flag'));
    t.emit(op.JR_FALSE, local('Flag'), label('Done'));
    t.emit(op.FILENAME, sub.FILENAME.EXIST, local('BakPath'), local('Flag'));
    t.emit(op.JR_EQ8, local('Flag'), local('BackOn'), label('Poll'));
    t.emit(op.MOVE8_8, local('Flag'), local('BackOn'));
    t.emit(op.UI_BUTTON, sub.UI_BUTTON.SET_BACK_BLOCK, local('BackOn'));
    t.emit(op.JR, label('Poll'));

    // Ending the thread ends the slot.
    t.label('Done');
    return t.assemble();
}

/** The launcher .rbf for a project, ready to upload as `<project>/<project>.rbf`. */
export function buildLauncher(options: LauncherOptions): Uint8Array {
    return assembleLauncher(options).bytes;
}
