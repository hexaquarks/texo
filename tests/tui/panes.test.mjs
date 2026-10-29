import assert from 'node:assert/strict';
import test from 'node:test';
import { createResume, prefix, withTexo } from './helpers.mjs';

test('TeX expansion keeps the PDF and tree and restores chat on navigation', async () => {
    await withTexo('editor-expansion', async ({ terminal }) => {
        await createResume(terminal, 'Pane layout');
        await prefix(terminal, 'u');
        await terminal.getByText('Restore').expect();
        await terminal.getByText('PDF current').expect();
        assert.ok(!(await terminal.text()).includes('texo / chat'));

        await prefix(terminal, 'j');
        await terminal.getByText('texo / chat').expect();
        await prefix(terminal, 'k');
        await prefix(terminal, 'u');
        await prefix(terminal, 'b');
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('Restore').expect();
        await prefix(terminal, 'u');
        await terminal.getByText('texo / chat').expect();
    });
});
