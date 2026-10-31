import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { ARTIFACTS, RESUME, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

async function replaceDocument(terminal, source) {
    await prefix(terminal, 'y');
    await terminal.press('w');
    await terminal.press('Enter');
    await terminal.press('Ctrl+a');
    await paste(terminal, source);
}

test('history previews, restores with recovery and survives restart', async () => {
    await withTexo('history-lifecycle', async ({ terminal, workspace, restart }) => {
        await terminal.resize(160, 45);
        await createResume(terminal, 'History');
        const document = await documentPath(workspace);
        const second = RESUME.replace('Terminal workflow', 'Second saved');
        const unsaved = RESUME.replace('Terminal workflow', 'Unsaved working');
        await replaceDocument(terminal, RESUME);
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        await replaceDocument(terminal, second);
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        await replaceDocument(terminal, unsaved);

        await prefix(terminal, 'y');
        await terminal.getByText('Unsaved changes').expect();
        await terminal.press('Down', 'Down', 'Enter');
        await terminal.getByText('read-only').expect();
        await terminal.getByText('Terminal workflow resume.').expect();
        await terminal.press('c');
        await terminal.getByText('including unsaved edits').expect({ not: true });
        await terminal.getByText('Unsaved working resume.').expect({ not: true });
        await terminal.getByText('Loading PDF…').expect({ not: true });
        await terminal.screenshot(join(ARTIFACTS, 'history-preview.png'));
        await prefix(terminal, 'c');
        await prefix(terminal, 'l');
        await prefix(terminal, 'z');
        await terminal.press('+');
        await terminal.getByText('120%').expect();
        await terminal.press('0');
        await terminal.getByText('120%').expect({ not: true });
        await prefix(terminal, 'z');
        await prefix(terminal, 'y');
        assert.equal(await readFile(document, 'utf8'), second);

        await terminal.press('m');
        await terminal.getByText('Restore').click();
        await terminal.getByText('Restore revision?').expect();
        await terminal.press('Escape');
        await terminal.getByText('Restore revision?').expect({ not: true });
        assert.equal(await readFile(document, 'utf8'), second);
        await terminal.press('m');
        await terminal.getByText('Restore').click();
        await terminal.getByText('Restore revision?').expect();
        await terminal.press('Enter');
        await terminal.getByText('Restore revision?').expect({ not: true });
        await terminal.getByText('revision-5').expect();
        assert.equal(await readFile(document, 'utf8'), RESUME);

        const historyPath = join(dirname(document), '.texo-history/main.tex.json');
        const journal = JSON.parse(await readFile(historyPath, 'utf8'));
        assert.equal(journal.revisions.length, 5);
        assert.equal(journal.revisions[3].kind, 'Recovery');
        assert.equal(journal.revisions[3].source, unsaved);
        assert.ok(journal.revisions.every(revision => !('name' in revision)));

        await restart();
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'y');
        await terminal.getByText('revision-4').expect();
        await terminal.getByText('Recovery', { whitespace: 'exact' }).expect({ not: true });
        await terminal.screenshot(join(ARTIFACTS, 'history-restarted.png'));
    }, { env: { NO_COLOR: '' } });
});

test('permanent history and resume switching preserve unsaved working copies', async () => {
    await withTexo('history-switching', async ({ terminal, workspace }) => {
        await createResume(terminal, 'First resume');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await terminal.getByText('History', { whitespace: 'exact' }).expect();
        await terminal.getByText('revision-1').expect();
        await terminal.getByText('⋯').first().click();
        await terminal.getByText('Restore').expect();
        await terminal.getByText('Rename', { whitespace: 'exact' }).expect({ not: true });
        await terminal.screenshot(join(ARTIFACTS, 'history-menu.png'));
        await terminal.getByText('Open', { whitespace: 'exact' }).click();
        await terminal.getByText('read-only').expect();
        await terminal.press('Escape');
        await prefix(terminal, 'r');
        await prefix(terminal, 'j');
        await terminal.press('Down', 'Enter');
        await terminal.getByText('read-only').expect();
        await prefix(terminal, 'k');
        await terminal.press('Enter');
        await terminal.getByText('read-only').expect({ not: true });
        await replaceDocument(terminal, RESUME);
        await terminal.getByText('Unsaved changes').expect();
        await terminal.getByText('revision-1').click();
        await terminal.getByText('read-only').expect();
        await prefix(terminal, 'c');
        await paste(terminal, 'MUST NOT EDIT');
        await terminal.press('x', 'Delete', 'Ctrl+s');
        assert.equal(await readFile(document, 'utf8'), saved);

        await createResume(terminal, 'Second resume');
        await terminal.getByText('First resume').first().click();
        await terminal.getByText('read-only').expect({ not: true });
        await terminal.getByText('Terminal workflow resume.').expect();
        await terminal.getByText('Unsaved changes').expect();
        await terminal.getByText('revision-1').click();
        await terminal.getByText('Your resume starts here.').expect();
        await prefix(terminal, 'y');
        await terminal.press('w', 'Enter');
        await terminal.getByText('Terminal workflow resume.').expect();
        assert.equal(await readFile(document, 'utf8'), saved);
        await terminal.resize(90, 30);
        await terminal.getByText('History', { whitespace: 'exact' }).expect();
        await terminal.screenshot(join(ARTIFACTS, 'history-narrow.png'));
    });
});
