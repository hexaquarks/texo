import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import test from 'node:test';
import { paste, prefix, withTexo } from './helpers.mjs';

test('Ctrl+J and raw LF add chat lines without submitting', async () => {
    await withTexo('chat-newline', async ({ terminal }) => {
        await prefix(terminal, 'j');
        await paste(terminal, 'First draft line');
        await terminal.press('Ctrl+j');
        await paste(terminal, 'Second draft line');

        // Traditional terminals encode Ctrl+J as LF; Enter sends CR in raw mode.
        await terminal.write('\n');
        await paste(terminal, 'Third draft line');
        await terminal.getByText('Third draft line').expect();

        const first = await terminal.getByText('First draft line').location();
        const second = await terminal.getByText('Second draft line').location();
        const third = await terminal.getByText('Third draft line').location();
        assert.equal(second.start.row, first.start.row + 1);
        assert.equal(third.start.row, second.start.row + 1);
    });
});

test('chat OS clipboard copy preserves the composer draft', {
    skip: process.env.TEXO_CLIPBOARD_GUARDED !== '1' ? 'Run test:clipboard to preserve the OS pasteboard' : false,
}, async () => {
    await withTexo('chat-clipboard', async ({ terminal }) => {
        await prefix(terminal, 'j');
        await paste(terminal, '/unknown');
        await terminal.press('Enter');
        await terminal.getByText('Unknown command.').expect();
        await paste(terminal, 'Draft text');
        await terminal.press('End');
        await prefix(terminal, 'z');
        await terminal.getByText('Restore').expect();

        execFileSync('pbcopy', { input: 'Clipboard seed' });
        const reply = await terminal.getByText('Unknown command.').location();
        await terminal.mouse.drag(reply.start.column, reply.start.row, reply.end.column, reply.end.row);
        await terminal.press('Ctrl+c');
        await terminal.getByText('Copied chat selection').expect();
        assert.equal(execFileSync('pbpaste', { encoding: 'utf8' }), 'Unknown command.');

        const word = await terminal.getByText('Unknown').location();
        await terminal.mouse.drag(word.start.column, word.start.row, word.end.column, word.end.row);
        await terminal.getByText('Unknown').and(terminal.getByStyle({ inverse: true })).expect();
        await terminal.press('Ctrl+Insert');
        await terminal.getByText('Copied chat selection').expect();
        assert.equal(execFileSync('pbpaste', { encoding: 'utf8' }), 'Unknown');

        await paste(terminal, ' stays here');
        await terminal.getByText('Draft text stays here').expect();
    });
});
