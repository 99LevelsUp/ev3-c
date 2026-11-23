import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { formatNoResult, formatReturned, formatRunResult, howItEnded } from '../shared/resultsView';

test('a run result shows how it ended, the PC start time, both outputs, and truncation', () => {
    const startedAt = new Date(2026, 10, 18, 10, 38, 5);
    const lines = formatRunResult('hello', {
        run: 3,
        record: { run: 3, end: 'program', exit: 2, outTruncated: true, errTruncated: false },
        out: 'line one\nline two\n',
        err: '',
        startedAt,
        returned: [],
    });
    assert.deepEqual(lines, [
        '--- hello, run 3 ---',
        `Started: ${startedAt.toLocaleString()} (PC time)`,
        'Ended: the program exited with code 2',
        'Output (stdout), truncated at the size limit:',
        '  line one',
        '  line two',
        'Errors (stderr): (empty)',
    ]);
});

test('each way a run can end has its own phrase, and a run started on the brick has no PC time', () => {
    const base = { run: 1, outTruncated: false, errTruncated: false };
    assert.equal(howItEnded({ ...base, end: 'program', signal: 11 }), 'the program was ended by signal 11');
    assert.equal(howItEnded({ ...base, end: 'slot', signal: 15 }), 'the program was stopped when its slot ended (signal 15)');
    assert.equal(howItEnded({ ...base, end: 'before-start' }), 'the slot ended before the program started');

    const lines = formatRunResult('hello', { run: 1, record: { ...base, end: 'program', exit: 0 }, returned: [] });
    assert.match(lines[1], /no PC start time/);
    assert.match(lines[3], /could not be read/);
    assert.match(formatNoResult('hello', 'abc', new Date())[1], /No result was recorded/);
    assert.match(formatNoResult('hello', 'abc', new Date(), '"ev3 c runs"')[1], /"ev3 c runs" lists the run/);
});

test('the files a run returned are listed with the result, and where they landed after', () => {
    const lines = formatRunResult('hello', {
        run: 2,
        record: { run: 2, end: 'program', exit: 0, outTruncated: false, errTruncated: false },
        out: '',
        err: '',
        returned: ['/home/root/lms2012/prjs/hello/log.csv'],
    });
    assert.deepEqual(lines.slice(-2), ['Returned files:', '  /home/root/lms2012/prjs/hello/log.csv']);
    assert.deepEqual(formatReturned({
        downloaded: [{ brickPath: '/a/x', localPath: 'C:\\p\\x' }],
        failed: [{ brickPath: '/a/y', reason: 'gone' }],
    }), ['Returned /a/x → C:\\p\\x', 'Could not return /a/y: gone']);
});
