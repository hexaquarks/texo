import test from 'node:test';
import { prefix, withTexo } from './helpers.mjs';

// Keep startup/input checks independent of the compiler so unsupported platforms
// still expose terminal failures while sandbox support is being implemented.
test('startup, shortcuts, resizing, and tree navigation work across terminal engines', async () => {
    await withTexo('platform-compatibility', async ({ terminal, restart }) => {
        await terminal.getByText('Hide tree').expect();
        await prefix(terminal, 't');
        await terminal.getByText('Show tree').expect();
        await terminal.getByText('No resumes yet').expect({ not: true });

        await prefix(terminal, '?');
        await terminal.getByText('Prefix: Ctrl-x').expect();
        await terminal.press('/', 'z', 'o', 'o', 'm');
        await terminal.getByText('Zoom pane').expect();
        await terminal.press('Escape');
        await terminal.press('Escape');

        await terminal.resize(60, 20);
        await prefix(terminal, 't');
        await terminal.getByText('No resumes yet').expect();
        await terminal.resize(120, 35);

        await prefix(terminal, 's');
        await terminal.getByText('WORKSPACE').expect();
        await terminal.press('Escape');
        await restart();
        await terminal.getByText('No resumes yet').expect();
        await terminal.getByText('Hide tree').expect();
        await terminal.press('Ctrl+q');
        await terminal.waitExit();
    });
});
