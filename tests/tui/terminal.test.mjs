import assert from 'node:assert/strict';
import { readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import test from 'node:test';
import {
    exportDestination, ARTIFACTS, RESUME, UI_TIMEOUT_MS, withTexo, paste, prefix, createResume, documentPath,
} from './helpers.mjs';

const executeFile = promisify(execFile);
const PDF_INSPECTION_TIMEOUT_MS = 10_000;

test('create, paste Unicode, save, build, and export through the terminal', async () => {
    await withTexo('edit-export', async ({ terminal, workspace, root }) => {
        await createResume(terminal, 'Résumé 東京');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await prefix(terminal, 'w');
        await terminal.getByText(' · Saved').expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), RESUME);

        await prefix(terminal, 'b');
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').first().expect();
        const exported = join(root, 'application.pdf');
        await exportDestination(terminal, exported);
        await terminal.press('Enter');
        await terminal.getByText('Exported').expect();
        const { stdout } = await executeFile('pdftotext', [exported, '-'], {
            timeout: PDF_INSPECTION_TIMEOUT_MS,
        });
        assert.match(stdout, /Terminal workflow resume/);
        await terminal.screenshot(join(ARTIFACTS, 'edit-export.png'));
    });
});

test('resize down to one cell, restore, and open settings', async () => {
    await withTexo('resize-menu', async ({ terminal }) => {
        await createResume(terminal, 'Resize test');
        for (const [cols, rows] of [[80, 24], [40, 12], [20, 5], [1, 1], [120, 35]]) {
            await terminal.resize(cols, rows);
            await terminal.waitIdle();
        }
        await terminal.getByText('Resize test').first().expect();
        await terminal.resize(40, 12);
        await prefix(terminal, 's');
        await terminal.getByText('WORKSPACE').expect();
        await terminal.screenshot(join(ARTIFACTS, 'small-settings.png'));
        await terminal.press('Escape');
        await terminal.resize(120, 35);
        await terminal.getByText('Resize test').first().expect();
    });
});

test('clicking the import name field directs typing into that field', async () => {
    await withTexo('import-mouse', async ({ terminal, root }) => {
        const sourceDirectory = join(root, 'source');
        await mkdir(sourceDirectory);
        const source = join(sourceDirectory, 'original.tex');
        await writeFile(source, RESUME);
        await prefix(terminal, 'i');
        await terminal.getByText('Choose a .tex file').expect();
        await paste(terminal, source);
        await terminal.getByText('Imported resume').click();
        await paste(terminal, 'Mouse imported');
        await terminal.press('Enter');
        await terminal.getByText('Choose the TeX entrypoint').expect();
        await terminal.press('Enter');
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('Mouse imported').first().expect();
        assert.equal(await readFile(source, 'utf8'), RESUME);
    });
});

test('mouse selection chooses the requested import entrypoint', async () => {
    await withTexo('entrypoint-mouse', async ({ terminal, root, workspace }) => {
        const source = join(root, 'source');
        await mkdir(source);
        await writeFile(join(source, 'first.tex'), RESUME);
        await writeFile(join(source, 'second.tex'), RESUME.replace('workflow', 'second entrypoint'));
        await prefix(terminal, 'i');
        await terminal.getByText('Choose a .tex file').expect();
        await paste(terminal, source);
        await terminal.press('Enter');
        await terminal.getByText('Choose the TeX entrypoint').expect();
        await terminal.getByText('second.tex').click();
        await terminal.press('Enter');
        await terminal.getByText('PDF current').expect();
        const projects = await readdir(workspace);
        const manifest = await readFile(join(workspace, projects[0], 'texo.toml'), 'utf8');
        assert.match(manifest, /entrypoint = "second\.tex"/);
    });
});

test('a scrolled import picker maps clicks after resizing', async () => {
    await withTexo('scrolled-entrypoints', async ({ terminal, root, workspace }) => {
        const source = join(root, 'source');
        await mkdir(source);
        const candidateCount = 12;
        for (let index = 0; index < candidateCount; index += 1) {
            const filename = `resume-${String(index).padStart(2, '0')}.tex`;
            await writeFile(join(source, filename), RESUME);
        }

        await prefix(terminal, 'i');
        await terminal.getByText('Choose a .tex file').expect();
        await paste(terminal, source);
        await terminal.press('Enter');
        await terminal.getByText('Choose the TeX entrypoint').expect();
        await terminal.resize(60, 14);
        const selectedIndex = 10;
        for (let index = 0; index < selectedIndex; index += 1) {
            await terminal.press('Down');
        }
        await terminal.getByText('resume-10.tex').expect();
        await terminal.getByText('resume-09.tex').click();
        await terminal.press('Enter');
        await terminal.resize(120, 35);
        await terminal.getByText('PDF current').expect();
        const projects = await readdir(workspace);
        const manifest = await readFile(join(workspace, projects[0], 'texo.toml'), 'utf8');
        assert.match(manifest, /entrypoint = "resume-09\.tex"/);
        await terminal.screenshot(join(ARTIFACTS, 'scrolled-import.png'));
    });
});

test('quit works with a modal open', async () => {
    await withTexo('quit-modal', async ({ terminal }) => {
        await prefix(terminal, 'n');
        await terminal.getByText('New resume').first().expect();
        await terminal.press('Ctrl+q');
        await terminal.waitExit({ timeout: UI_TIMEOUT_MS });
        const state = await terminal.state();
        assert.equal(state.exited, 0);
        assert.equal(state.modes.bracketed_paste, false);
    });
});

test('quit confirmation protects unsaved text and restores terminal modes after discard', async () => {
    await withTexo('quit-unsaved', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Quit protection');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await paste(terminal, 'Unsaved edit ');
        await terminal.press('Ctrl+q');
        await terminal.getByText('Quit Texo?').expect();
        await terminal.getByText('unsaved text').expect();
        await terminal.screenshot(join(ARTIFACTS, 'quit-confirmation.png'));

        await terminal.press('Ctrl+q');
        await terminal.getByText('Quit Texo?').expect();
        await terminal.press('Enter');
        await terminal.getByText('Quit Texo?').expect({ not: true });
        await terminal.getByText('Unsaved edit').expect();
        assert.equal(await readFile(document, 'utf8'), saved);

        await terminal.press('Ctrl+q');
        await terminal.getByText('Quit Texo?').expect();
        await terminal.press('Right');
        await terminal.press('Enter');
        await terminal.waitExit({ timeout: UI_TIMEOUT_MS });
        const state = await terminal.state();
        assert.equal(state.exited, 0);
        assert.equal(state.modes.bracketed_paste, false);
        assert.equal(await readFile(document, 'utf8'), saved);
    }, { env: { NO_COLOR: '' } });
});

test('quit confirmation identifies unsaved work in a different resume', async () => {
    await withTexo('quit-cached-resume', async ({ terminal }) => {
        await createResume(terminal, 'A backend resume');
        await paste(terminal, 'Unsaved backend edit ');
        await createResume(terminal, 'B research resume');
        await terminal.press('Ctrl+q');
        await terminal.getByText('Quit Texo?').expect();
        await terminal.getByText('A backend resume (').expect();
        await terminal.getByText('unsaved text').expect();
        await terminal.screenshot(join(ARTIFACTS, 'quit-cached-resume.png'));

        await terminal.press('Esc');
        await prefix(terminal, 'r');
        await terminal.press('Up');
        await terminal.getByText('Unsaved backend edit').expect();
    }, { env: { NO_COLOR: '' } });
});
