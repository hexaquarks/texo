import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { ARTIFACTS, RESUME, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

async function replaceDocument(terminal, source) {
    await prefix(terminal, 'f');
    await terminal.press('Enter');
    await terminal.press('Ctrl+a');
    await paste(terminal, source);
}

test('history previews, compares, restores with recovery, names revisions, and survives restart', async () => {
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
        await terminal.getByText('Working copy * unsaved').expect();
        await terminal.press('Down', 'Down', 'Enter');
        await terminal.getByText('read-only').expect();
        await terminal.getByText('Terminal workflow resume.').expect();
        await terminal.press('c');
        await terminal.getByText('including unsaved edits').expect();
        await terminal.getByText('Unsaved working resume.').expect();
        await terminal.getByText('Loading PDF…').expect({ not: true });
        await terminal.screenshot(join(ARTIFACTS, 'history-comparison.png'));
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

        await terminal.press('r');
        await terminal.getByText('Restore revision?').expect();
        await terminal.press('Escape');
        await terminal.getByText('Restore revision?').expect({ not: true });
        assert.equal(await readFile(document, 'utf8'), second);
        await terminal.press('r');
        await terminal.getByText('Restore revision?').expect();
        await terminal.press('Enter');
        await terminal.getByText('Restore revision?').expect({ not: true });
        await terminal.getByText('Restored', { whitespace: 'exact' }).expect();
        assert.equal(await readFile(document, 'utf8'), RESUME);

        const historyPath = join(dirname(document), '.texo-history/main.tex.json');
        const journal = JSON.parse(await readFile(historyPath, 'utf8'));
        assert.equal(journal.revisions.length, 5);
        assert.equal(journal.revisions[3].kind, 'Recovery');
        assert.equal(journal.revisions[3].source, unsaved);
        await terminal.press('Down', 'n');
        await terminal.getByText('Name revision').expect();
        await paste(terminal, 'Application ready');
        await terminal.press('Enter');
        await terminal.getByText('Name revision').expect({ not: true });
        await terminal.getByText('Application ready').expect();
        const renamed = JSON.parse(await readFile(historyPath, 'utf8'));
        assert.equal(renamed.revisions.length, 5);
        assert.equal(renamed.revisions.at(-1).name, 'Application ready');

        await restart();
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'y');
        await terminal.getByText('Application ready').expect();
        await terminal.getByText('Recovery', { whitespace: 'exact' }).expect();
        await terminal.screenshot(join(ARTIFACTS, 'history-restarted.png'));
    }, { env: { NO_COLOR: '' } });
});

test('included TeX files have independent history and browsing cannot edit the live file', async () => {
    await withTexo('history-files', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Included files');
        const document = await documentPath(workspace);
        const other = join(dirname(document), 'other.tex');
        await writeFile(other, RESUME.replace('Terminal workflow', 'Included file'));
        await prefix(terminal, 'v');
        await terminal.getByText('other.tex').click();
        await terminal.getByText('Included file resume.').expect();
        await prefix(terminal, 'y');
        await terminal.press('Down', 'Enter');
        await terminal.getByText('read-only').expect();
        await prefix(terminal, 'c');
        await paste(terminal, 'MUST NOT EDIT');
        await terminal.press('x', 'Delete', 'Ctrl+s');
        assert.equal(await readFile(other, 'utf8'), RESUME.replace('Terminal workflow', 'Included file'));
        await terminal.press('Escape');
        await prefix(terminal, 'f');
        await terminal.getByText('main.tex').click();
        await prefix(terminal, 'y');
        await terminal.press('Down', 'Enter');
        await terminal.getByText('Your resume starts here.').expect();
        await terminal.getByText('Included file resume.').expect({ not: true });
    });
});
