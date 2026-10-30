import assert from 'node:assert/strict';
import { join } from 'node:path';
import test from 'node:test';
import { ARTIFACTS, createResume, prefix, withTexo } from './helpers.mjs';

test('prefix keeps the footer stable and opens a scrollable shortcut list', async () => {
    await withTexo('shortcut-help', async ({ terminal }) => {
        await createResume(terminal, 'Shortcuts');
        await terminal.getByText('Ctrl-x ?').expect();
        const footer = (await terminal.text()).trimEnd().split('\n').at(-1);
        await terminal.press('Ctrl+x');
        await terminal.waitIdle();
        assert.equal((await terminal.text()).trimEnd().split('\n').at(-1), footer);

        await terminal.press('?');
        await terminal.getByText('Shortcuts · Ctrl-x').expect();
        await terminal.getByText('Zoom pane').expect();
        await terminal.screenshot(join(ARTIFACTS, 'shortcut-help.png'));
        await terminal.press('w');
        await terminal.getByText('Shortcuts · Ctrl-x').expect();

        await terminal.resize(50, 12);
        await terminal.press('End');
        await terminal.getByText('Settings').expect();
        await terminal.press('Escape');
        await terminal.getByText('Shortcuts · Ctrl-x').expect({ not: true });
        await terminal.resize(120, 35);
        await prefix(terminal, 's');
        await terminal.getByText('Resume folder').expect();
        await terminal.press('Escape');
    }, { env: { NO_COLOR: '' } });
});
