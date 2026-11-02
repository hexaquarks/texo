import assert from 'node:assert/strict';
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
