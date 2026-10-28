import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { ARTIFACTS, RESUME, createResume, documentPath, paste, prefix, replaceField, withTexo } from './helpers.mjs';

const TWO_PAGES = '\\documentclass{article}\n\\begin{document}\n\\Huge First page\\newpage Second page\n\\end{document}\n';

test('PDF navigation, zoom, pan, drag, and reset remain usable across resizing and overlays', async () => {
    await withTexo('preview-navigation', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Preview');
        await terminal.press('Ctrl+a');
        await paste(terminal, TWO_PAGES);
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        await prefix(terminal, 'b');
        await terminal.getByText('1 / 2').expect();
        await terminal.getByText('PDF current').click();
        await terminal.press('Alt+Right');
        await terminal.getByText('2 / 2').expect();
        await terminal.press('Alt+Right');
        await terminal.press('Alt+Left');
        await terminal.getByText('1 / 2').expect();
        await terminal.press('+');
        await terminal.getByText('120%').expect();
        await terminal.press('+', '+', '+', '+');
        await terminal.getByText('200%').expect();
        await terminal.press('Right', 'Alt+Down');
        await terminal.getByText('PDF current').expect();
        // The preview occupies the right pane at this fixed 120-by-35 viewport.
        await terminal.mouse.drag(90, 16, 85, 12);
        await terminal.screenshot(join(ARTIFACTS, 'preview-drag.png'));
        await prefix(terminal, 's');
        await terminal.getByText('Resume folder').expect();
        await terminal.resize(60, 16);
        await terminal.press('Escape');
        await terminal.resize(120, 35);
        await terminal.getByText('200%').expect();
        await terminal.getByText('PDF current').click();
        await terminal.press('0');
        await terminal.getByText('1 / 2 · [1-9][0-9]?%', { regex: true }).expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), TWO_PAGES);
    });
});

test('failed build preserves the published PDF, refuses export, and recovers through the editor', async () => {
    await withTexo('build-recovery', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Build recovery');
        const document = await documentPath(workspace);
        const pdf = document.replace(/\.tex$/, '.pdf');
        const original = await readFile(pdf);
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME.replace('Terminal workflow resume.', '\\undefinedTestCommand'));
        await prefix(terminal, 'b');
        await terminal.getByText('Undefined control sequence').expect();
        assert.deepEqual(await readFile(pdf), original);
        await prefix(terminal, 'e');
        await terminal.getByText('latest build failed').expect();
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await prefix(terminal, 'b');
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').expect();
        await terminal.press('Escape');
    });
});

test('export rejects its build output and missing folders, then requires overwrite confirmation', async () => {
    await withTexo('export-conflicts', async ({ terminal, workspace, root }) => {
        await createResume(terminal, 'Export conflicts');
        const document = await documentPath(workspace);
        const pdf = document.replace(/\.tex$/, '.pdf');
        const original = await readFile(pdf);
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').expect();
        await replaceField(terminal, pdf);
        await terminal.press('Enter');
        await terminal.getByText('different from the build output').expect();
        await replaceField(terminal, join(root, 'missing/application.pdf'));
        await terminal.press('Enter');
        await terminal.getByText('directory does not exist').expect();

        const exported = join(root, 'existing.pdf');
        await writeFile(exported, 'Previous application');
        await replaceField(terminal, exported);
        await terminal.press('Enter');
        await terminal.getByText('File exists.').expect();
        assert.equal(await readFile(exported, 'utf8'), 'Previous application');
        await terminal.press('Escape');
        assert.equal(await readFile(exported, 'utf8'), 'Previous application');
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').expect();
        await replaceField(terminal, exported);
        await terminal.press('Enter');
        await terminal.getByText('File exists.').expect();
        await terminal.press('Enter');
        await terminal.getByText('Exported').expect();
        assert.deepEqual(await readFile(exported), original);
        assert.deepEqual(await readFile(pdf), original);
    });
});
