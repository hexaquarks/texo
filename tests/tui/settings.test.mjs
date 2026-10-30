import assert from 'node:assert/strict';
import { mkdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { ARTIFACTS, RESUME, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

test('grouped preferences stage toggles and protect work across apply and cancel', async () => {
    await withTexo('grouped-settings', async ({ terminal, root, workspace }) => {
        await createResume(terminal, 'Preferences');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await prefix(terminal, 's');
        await terminal.getByText('EDITOR').expect();
        await terminal.getByText('WORKSPACE').expect();
        await terminal.getByText('ADVANCED').expect();
        await terminal.press('l', 'j', 'Space');
        await terminal.getByText('Disabled').expect();
        await terminal.getByText('Preference changes are not applied yet.').expect();
        await terminal.screenshot(join(ARTIFACTS, 'settings-grouped.png'));
        await terminal.getByText('Reload config').click();
        await terminal.getByText('Apply or cancel preference changes').expect();
        await terminal.getByText('Change…').click();
        await terminal.getByText('Apply or cancel preference changes').expect();
        await terminal.press('Escape');
        await terminal.getByText('Terminal workflow resume.').expect();

        await prefix(terminal, 's');
        await terminal.getByText('Enabled').expect();
        await terminal.press('Tab', 'Space');
        await terminal.getByText('Apply changes').click();
        await terminal.getByText('EDITOR').expect({ not: true });
        assert.match(await readFile(join(root, 'config/texo/config.toml'), 'utf8'), /soft_wrap = false/);
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), RESUME);
    }, { env: { NO_COLOR: '' } });
});

test('workspace switch defaults to cancel and names only actual session loss', async () => {
    await withTexo('settings-switch-confirmation', async ({ terminal, root, workspace }) => {
        await createResume(terminal, 'Keep work');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        const destination = join(root, 'destination');
        await mkdir(destination);
        await prefix(terminal, 's');
        await terminal.getByText('Change…').click();
        await paste(terminal, destination);
        await terminal.press('Enter');
        await terminal.getByText('This discards: unsaved text.').expect();
        await terminal.getByText('Switch & discard').expect();
        await terminal.screenshot(join(ARTIFACTS, 'settings-switch.png'));
        await terminal.press('Enter');
        await terminal.getByText('EDITOR').expect();
        await terminal.press('Escape');
        await terminal.getByText('Terminal workflow resume.').expect();
        assert.equal(await readFile(document, 'utf8'), saved);
    }, { env: { NO_COLOR: '' } });
});
