import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFile, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import test from 'node:test';
import {
    ARTIFACTS, RESUME, action, createResume, documentPath, paste, prefix, replaceField, withTexo,
} from './helpers.mjs';

async function manifestAt(document) {
    return readFile(join(dirname(document), 'texo.toml'), 'utf8');
}

async function changeName(terminal, command, name) {
    await action(terminal, command.replace(' resume', ''));
    await paste(terminal, name);
    await terminal.press('Enter');
    await terminal.getByText(command).expect({ not: true });
    await terminal.getByText(name).first().expect();
}

test('rename and duplicate preserve unsaved text, supporting files, and project identity', async () => {
    await withTexo('project-management', async ({ terminal, workspace, restart }) => {
        await createResume(terminal, 'Original');
        const original = await documentPath(workspace);
        const originalManifest = await manifestAt(original);
        const saved = await readFile(original, 'utf8');
        await writeFile(join(dirname(original), 'asset.txt'), 'Supporting asset');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);

        await changeName(terminal, 'Rename resume', 'Renamed 東京');
        assert.match(await manifestAt(original), /name = "Renamed 東京"/);
        assert.equal((await manifestAt(original)).split('\n')[0], originalManifest.split('\n')[0]);
        assert.equal(await readFile(original, 'utf8'), saved);

        await changeName(terminal, 'Duplicate resume', 'Copy');
        await terminal.getByText('PDF current').expect();
        const folders = await readdir(workspace, { withFileTypes: true });
        const copyFolder = folders.find(entry => entry.isDirectory() && join(workspace, entry.name) !== dirname(original));
        const copy = join(workspace, copyFolder.name, 'main.tex');
        assert.equal(await readFile(copy, 'utf8'), RESUME);
        assert.equal(await readFile(join(dirname(copy), 'asset.txt'), 'utf8'), 'Supporting asset');
        assert.notEqual((await manifestAt(copy)).split('\n')[0], originalManifest.split('\n')[0]);

        await terminal.getByText('Renamed 東京').first().click();
        await prefix(terminal, 'w');
        await terminal.getByText(' · Saved').expect();
        assert.equal(await readFile(original, 'utf8'), RESUME);
        await restart();
        await terminal.getByText('Renamed 東京').first().expect();
        await terminal.getByText('Copy').first().expect();
        await terminal.getByText('PDF current').expect();
    });
});

async function rebuildResume(terminal, text) {
    await terminal.press('Ctrl+a');
    await paste(terminal, RESUME.replace('Terminal workflow resume.', text));
    await terminal.getByText(text).expect();

    await prefix(terminal, 'w');
    await terminal.getByText('unsaved').expect({ not: true });

    await prefix(terminal, 'b');
    await terminal.getByText('PDF current').expect();
}

async function exportWithDefaultName(terminal, name) {
    await prefix(terminal, 'e');
    await terminal.getByText(name).expect();
    await terminal.waitIdle();
    await terminal.screenshot(join(ARTIFACTS, name.replace('.pdf', '.png')));

    await terminal.press('Enter');
    await terminal.getByText('Export PDF', { whitespace: 'exact' }).expect({ not: true });
}

test('duplicate resumes keep PDF assets and export only the edited variant', async () => {
    await withTexo('duplicate-exports', async ({ terminal, workspace }) => {
        await createResume(terminal, 'River Original');
        const original = dirname(await documentPath(workspace));
        await rebuildResume(terminal, 'River Original. Field research.');
        const originalName = 'River-Original-export.pdf';
        await exportWithDefaultName(terminal, originalName);

        const originalExport = join(original, originalName);
        const originalBytes = await readFile(originalExport);
        const originalText = execFileSync('pdftotext', [originalExport, '-'], { encoding: 'utf8' });
        assert.match(originalText, /Field research/);

        // Recreate the unmarked default from earlier Texo versions and an asset with identical bytes.
        await copyFile(originalExport, join(original, 'main-export.pdf'));
        await copyFile(originalExport, join(original, 'portfolio-export.pdf'));

        await changeName(terminal, 'Duplicate resume', 'River Variant');
        await terminal.getByText('PDF current').expect();

        const variant = join(workspace, 'river-variant');
        await assert.rejects(readFile(join(variant, originalName)), { code: 'ENOENT' });
        await assert.rejects(readFile(join(variant, 'main-export.pdf')), { code: 'ENOENT' });
        assert.deepEqual(await readFile(join(variant, 'portfolio-export.pdf')), originalBytes);

        await rebuildResume(terminal, 'River Variant. Ocean engineering.');
        const variantName = 'River-Variant-export.pdf';
        await exportWithDefaultName(terminal, variantName);

        const variantExport = join(variant, variantName);
        const variantText = execFileSync('pdftotext', [variantExport, '-'], { encoding: 'utf8' });
        assert.match(variantText, /Ocean engineering/);
        assert.doesNotMatch(variantText, /Field research/);
        assert.deepEqual(await readFile(variantExport), await readFile(join(variant, 'main.pdf')));
        assert.deepEqual(await readFile(originalExport), originalBytes);
    });
});

async function importResume(terminal, path, name) {
    await prefix(terminal, 'i');
    await terminal.getByText('Choose a .tex file').expect();
    await paste(terminal, path);
    await terminal.getByText('Imported resume').click();
    await paste(terminal, name);
    await terminal.press('Enter');

    await terminal.getByText('Choose the TeX entrypoint').expect();
    await terminal.press('Enter');
    await terminal.getByText(name).first().expect();
    await terminal.getByText('PDF current').expect();
    await terminal.waitIdle();
}

test('the default export stays distinct from the build, including after a managed import', async () => {
    await withTexo('managed-import-exports', async ({ terminal, root, workspace }) => {
        const source = join(root, 'source');
        await mkdir(source);
        await writeFile(join(source, 'River-export.tex'), RESUME);
        await importResume(terminal, source, 'River');

        const original = join(workspace, 'river');
        const build = join(original, 'River-export.pdf');
        const buildBytes = await readFile(build);
        await exportWithDefaultName(terminal, 'River-export-1.pdf');

        const exported = join(original, 'River-export-1.pdf');
        assert.deepEqual(await readFile(exported), buildBytes);
        assert.deepEqual(await readFile(build), buildBytes);
        const pdfText = execFileSync('pdftotext', [exported, '-'], { encoding: 'utf8' });
        assert.match(pdfText, /Terminal workflow resume/);
        await copyFile(exported, join(original, 'portfolio.pdf'));

        await importResume(terminal, original, 'Imported');
        await changeName(terminal, 'Duplicate resume', 'Variant');
        await terminal.getByText('PDF current').expect();

        for (const name of ['imported', 'variant']) {
            const project = join(workspace, name);
            await assert.rejects(readFile(join(project, 'River-export-1.pdf')), { code: 'ENOENT' });
            assert.deepEqual(await readFile(join(project, 'portfolio.pdf')), buildBytes);
        }
    });
});

test('labels validate custom input, persist, reuse choices, and can be removed', async () => {
    await withTexo('labels', async ({ terminal, workspace, restart }) => {
        await createResume(terminal, 'Labelled');
        const document = await documentPath(workspace);
        await action(terminal, 'Change label');
        await terminal.getByText('Custom…').click();
        await terminal.getByText('Custom label').expect();
        await paste(terminal, 'one two three four five');
        await terminal.press('Enter');
        await terminal.getByText('Use 1–3 words').expect();
        assert.doesNotMatch(await manifestAt(document), /\[label\]/);
        await terminal.press('Ctrl+a');
        await paste(terminal, 'Backend');
        await terminal.press('Tab', 'Right', 'Enter');
        await terminal.getByText('Resume label').expect({ not: true });
        assert.match(await manifestAt(document), /name = "Backend"/);

        await restart();
        await terminal.getByText('Backend').first().expect();
        await action(terminal, 'Change label');
        await terminal.getByText('No label').click();
        await terminal.getByText('Resume label').expect({ not: true });
        assert.doesNotMatch(await manifestAt(document), /\[label\]/);
        await action(terminal, 'Change label');
        await terminal.getByText('Backend').click();
        await terminal.getByText('Resume label').expect({ not: true });
        assert.match(await manifestAt(document), /name = "Backend"/);
    });
});

test('external edits block saving, reload safely, and deleted source can be restored or cancelled', async () => {
    await withTexo('source-recovery', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Recovery');
        const document = await documentPath(workspace);
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        const external = RESUME.replace('Terminal workflow', 'External edit');
        await writeFile(document, external);
        await prefix(terminal, 'w');
        await terminal.getByText('Source changed').expect();
        assert.equal(await readFile(document, 'utf8'), external);
        await terminal.getByText('Error details').expect();
        await terminal.press('Escape');
        await action(terminal, 'Reload from disk');
        await terminal.getByText('Unsaved text changes will be discarded').expect();
        await terminal.press('Escape');
        await terminal.getByText('Terminal workflow resume.').expect();
        await action(terminal, 'Reload from disk');
        await terminal.press('Right', 'Enter');
        await terminal.getByText('External edit resume.').expect();
        await terminal.getByText('PDF current').expect();

        await rm(document);
        await prefix(terminal, 'w');
        await terminal.getByText('Restore missing source').expect();
        await terminal.press('Escape');
        await assert.rejects(readFile(document), { code: 'ENOENT' });
        await prefix(terminal, 'w');
        await terminal.getByText('Restore missing source').expect();
        await terminal.press('Right', 'Enter');
        await terminal.getByText('Restore missing source').expect({ not: true });
        assert.equal(await readFile(document, 'utf8'), external);
        await terminal.getByText('PDF current').expect();
    });
});

test('settings reject unavailable folders, switch workspaces, and persist mode across restart', async () => {
    await withTexo('settings-workspace', async ({ terminal, root, workspace, restart }) => {
        await createResume(terminal, 'Keep saved');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await prefix(terminal, 's');
        await terminal.getByText('WORKSPACE').expect();
        await terminal.getByText('Vim', { whitespace: 'exact' }).click();
        await terminal.getByText('Apply changes').click();
        await prefix(terminal, 's');
        await terminal.getByText('Change…').click();
        await paste(terminal, join(root, 'missing'));
        await terminal.press('Enter');
        await terminal.getByText('Choose an existing folder').expect();
        assert.equal(await readFile(document, 'utf8'), saved);

        const other = join(root, 'other');
        await mkdir(other);
        await replaceField(terminal, other);
        await terminal.getByText('Continue').click();
        await terminal.getByText('Switch workspace').click();
        await terminal.getByText('No resumes yet').expect();
        const configPath = join(root, 'config/texo/config.toml');
        assert.match(await readFile(configPath, 'utf8'), /mode = "vim"/);
        await restart({ configuredWorkspace: true });
        await terminal.getByText('No resumes yet').expect();
        await createResume(terminal, 'Other workspace');
        await terminal.getByText('NORMAL').expect();
        assert.equal((await readdir(other)).length, 1);
        assert.equal(await readFile(document, 'utf8'), saved);
    });
});

test('config reload rejects malformed settings and updates the prefix without losing unsaved text', async () => {
    await withTexo('config-reload', async ({ terminal, root }) => {
        await createResume(terminal, 'Config test');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await prefix(terminal, 's');
        await terminal.getByText('WORKSPACE').expect();
        const configPath = join(root, 'config/texo/config.toml');
        await mkdir(dirname(configPath), { recursive: true });
        await writeFile(configPath, 'broken = [');
        await terminal.press('Ctrl+r');
        await terminal.getByText('Invalid config.toml').expect();
        await terminal.press('Escape');
        await terminal.getByText('Terminal workflow resume.').expect();
        await prefix(terminal, 's');
        await terminal.getByText('WORKSPACE').expect();
        const workspace = join(root, 'resumes');
        await writeFile(configPath, `resume_folder = ${JSON.stringify(workspace)}\n[keybindings]\nprefix = "ctrl+g"\n`);
        await terminal.press('Ctrl+r');
        await terminal.getByText('Configuration reloaded.').expect();
        await terminal.press('Escape');
        await terminal.press('Ctrl+g', 'w');
        await terminal.getByText(' · Saved').expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), RESUME);
    });
});

test('a TeX file argument opens the document directly', async () => {
    await withTexo('document-argument', async ({ terminal, root, restart }) => {
        const document = join(root, 'resume.tex');
        await writeFile(document, RESUME);

        await restart({ path: document });
        await terminal.getByText('resume.tex').first().expect();
        await terminal.getByText('Terminal workflow resume.').expect();
        await terminal.getByText('PDF current').expect();
    });
});

test('Manage deletes only after confirmation and preserves saved files in the workspace trash', async () => {
    await withTexo('delete-resume', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Keep me');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);

        await createResume(terminal, 'Remove me');
        const removed = join(workspace, 'remove-me');
        const source = join(removed, 'main.tex');
        const saved = await readFile(source, 'utf8');

        await action(terminal, 'Delete');
        await terminal.getByText('Delete resume?').expect();
        await terminal.press('Enter');
        await terminal.getByText('Delete resume?').expect({ not: true });
        assert.equal(await readFile(source, 'utf8'), saved);

        await action(terminal, 'Delete');
        await terminal.getByText('Delete resume?').expect();
        await terminal.press('Right', 'Enter');
        await terminal.getByText('Terminal workflow resume.').expect();

        const trash = join(workspace, '.texo-trash');
        const entries = await readdir(trash);
        assert.equal(entries.length, 1);
        assert.equal(await readFile(join(trash, entries[0], 'remove-me', 'main.tex'), 'utf8'), saved);
        await assert.rejects(readFile(source), { code: 'ENOENT' });
    });
});
