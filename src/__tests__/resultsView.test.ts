import { test } from 'node:test';
import * as assert from 'node:assert/strict';
import { formatNoResult, formatReturned, formatRunResult, summaryLabel } from '../host/resultsView';

const record = (ended: string) => ({ end: 'program', exit: 2, outTruncated: true, errTruncated: false, ended });

test('a run result shows how it ended, the PC start time, both outputs, and truncation', () => {
    const startedAt = new Date(2026, 10, 18, 10, 38, 5);
    const lines = formatRunResult('hello', {
        run: 3,
        record: record('the program exited with code 2'),
        out: 'line one\nline two\n',
        err: '',
        startedAt: startedAt.toISOString(),
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

test('a run started elsewhere has no PC time, and an output not read says so', () => {
    const lines = formatRunResult('hello', { run: 1, record: { ...record('the program ended'), outTruncated: false }, returned: [] });
    assert.match(lines[1], /no PC start time/);
    assert.match(lines[3], /could not be read/);
    assert.match(formatNoResult('hello', 'abc', new Date().toISOString())[1], /No result was recorded on the brick \(launch abc\)/);
    assert.deepEqual(summaryLabel({ run: 4 }), { label: 'Run 4', description: 'no completion record (running, or interrupted)' });
    assert.match(summaryLabel({ run: 5, record: record('the program exited with code 2') }).description,
        /^the program exited with code 2; started not started by EVƎ C/);
});

test('the files a run returned are listed with the result, and where they landed after', () => {
    const lines = formatRunResult('hello', {
        run: 2,
        record: record('the program exited with code 0'),
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
