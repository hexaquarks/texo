import test from 'node:test';
import { createResume, prefix, withTexo } from './helpers.mjs';

test('TeX expansion keeps the PDF and tree and restores chat on navigation', async () => {
    await withTexo('editor-expansion', async ({ terminal }) => {
        await terminal.resize(160, 45);
        await createResume(terminal, 'Pane layout');
        await prefix(terminal, 'u');
        await terminal.getByText('Restore').expect();
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('texo / chat').expect({ not: true });

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

test('workspace expansion restores on build and PDF controls use the enlarged pane', async () => {
    await withTexo('workspace-expansion', async ({ terminal }) => {
        await createResume(terminal, 'Workspace layout');
        await prefix(terminal, 'z');
        await terminal.getByText('PDF current').expect({ not: true });
        await prefix(terminal, 'b');
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('texo / chat').expect();

        await terminal.getByText('PDF current').click();
        await prefix(terminal, 'z');
        await terminal.getByText('texo / chat').expect({ not: true });
        await terminal.press('+');
        await terminal.getByText('120%').expect();
        await terminal.press('Right', 'Alt+Down');
        // This point belonged to TeX/chat before expansion; it must now drag the PDF.
        await terminal.mouse.drag(40, 16, 35, 12);
        await terminal.press('0');
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'z');
        await terminal.getByText('texo / chat').expect();
    });
});
