import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import {
    ARTIFACTS, RESUME, action, createResume, documentPath, exportDestination, paste, prefix, withTexo,
} from './helpers.mjs';

test('project dialogs share visible controls and preserve work when cancelled', async () => {
    await withTexo('project-dialogs', async ({ terminal, root, workspace }) => {
        await createResume(terminal, 'AI Engineering');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await terminal.getByText('Manage', { whitespace: 'exact' }).click();
        await terminal.getByText('Manage resume').expect();
        assert.doesNotMatch(await terminal.text(), /Label:/);
        await terminal.press('j', 'j');
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-manage.png'));
        await terminal.press('Escape');

        await action(terminal, 'Change label');
        await terminal.getByText('› No label').expect();
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-labels.png'));
        await terminal.press('j');
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-labels-moved.png'));
        await terminal.press('Escape');

        await action(terminal, 'Reload from disk');
        await terminal.getByText('The on-disk file will not be changed.').expect();
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-reload-clean.png'));
        await terminal.press('Enter');
        await terminal.getByText('Reload from disk?').expect({ not: true });

        for (const name of ['Rename', 'Duplicate']) {
            await action(terminal, name);
            await terminal.getByText('Resume name').expect();
            await terminal.waitIdle();
            await terminal.screenshot(join(ARTIFACTS, `dialog-${name.toLowerCase()}.png`));
            await terminal.press('Tab');
            await terminal.getByText('› Cancel').expect();
            await terminal.waitIdle();
            await terminal.screenshot(join(ARTIFACTS, `dialog-${name.toLowerCase()}-cancel-focus.png`));
            await terminal.press('Tab');
            await terminal.getByText(`› ${name}`).expect();
            await terminal.waitIdle();
            await terminal.screenshot(join(ARTIFACTS, `dialog-${name.toLowerCase()}-action-focus.png`));
            await terminal.press('Shift+Tab', 'Enter');
            await terminal.getByText(`${name} resume`).expect({ not: true });
        }

        await prefix(terminal, 'e');
        await exportDestination(terminal, join(root, 'application.pdf'));
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-export.png'));
        await terminal.resize(40, 16);
        await terminal.waitIdle();
        await terminal.getByText('application.pdf').expect();
        await terminal.press('Tab', 'Enter');
        await terminal.resize(120, 35);
        await terminal.waitIdle();
        await assert.rejects(readFile(join(root, 'application.pdf')), { code: 'ENOENT' });

        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await action(terminal, 'Reload from disk');
        await terminal.getByText('Unsaved text changes will be discarded.').expect();
        assert.doesNotMatch(await terminal.text(), /pending AI proposal/);
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-reload.png'));
        await terminal.press('Enter');
        await terminal.getByText('Terminal workflow resume.').expect();
        assert.equal(await readFile(document, 'utf8'), saved);

        const source = join(root, 'source');
        await mkdir(source);
        await writeFile(join(source, 'main.tex'), RESUME);
        await prefix(terminal, 'i');
        await paste(terminal, source);
        await terminal.press('Tab');
        await paste(terminal, 'Imported');
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-import.png'));
        await terminal.getByText('Review import').click();
        await terminal.getByText('Choose the TeX entrypoint').expect();
        assert.equal((await readdir(workspace)).length, 1);
        await terminal.press('Escape');
        await terminal.getByText('Review import').expect();
        await terminal.getByText('Cancel', { whitespace: 'exact' }).click();

        await prefix(terminal, 's');
        await terminal.getByText('Change…').click();
        await terminal.getByText('Switch resume folder').expect();
        await terminal.waitIdle();
        await terminal.screenshot(join(ARTIFACTS, 'dialog-switch-folder.png'));
        await terminal.getByText('Cancel', { whitespace: 'exact' }).click();
        await terminal.getByText('EDITOR').expect();
        await terminal.press('Escape');
        await terminal.getByText('Terminal workflow resume.').expect();
    }, { env: { NO_COLOR: '' } });
});
