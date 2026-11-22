import * as vscode from 'vscode';

// Questions EVƎ Cockpit's daemon puts to a person through EVƎ C, for operations EVƎ C
// started: mostly `sweep`, when an earlier transfer left a file slot open on the brick.
// VS Code cannot close a notification, so a question the daemon withdraws leaves it on
// screen and answers `keep`, which the daemon reads as unanswered.

export type Answer = 'yes' | 'no' | 'keep';

export interface PersonQuestion {
    readonly text: string;
    readonly details?: readonly string[];
}

export async function askPerson(question: PersonQuestion, signal: AbortSignal): Promise<Answer> {
    if (signal.aborted) {
        return 'keep';
    }
    const detail = question.details && question.details.length > 0 ? `\n${question.details.join('\n')}` : '';
    const withdrawn = new Promise<undefined>(resolve => signal.addEventListener('abort', () => resolve(undefined), { once: true }));
    const picked = await Promise.race([
        vscode.window.showWarningMessage(`EVƎ Cockpit: ${question.text}${detail}`, 'Yes', 'No'),
        withdrawn,
    ]);
    if (signal.aborted) {
        return 'keep';
    }
    return picked === 'Yes' ? 'yes' : picked === 'No' ? 'no' : 'keep';
}
