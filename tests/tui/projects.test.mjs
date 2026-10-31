import assert from 'node:assert/strict';
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { RESUME, action, createResume, documentPath, paste, prefix, replaceField, withTexo } from './helpers.mjs';

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
        await terminal.getByText('Saved TeX document.').expect();
        assert.equal(await readFile(original, 'utf8'), RESUME);
        await restart();
        await terminal.getByText('Renamed 東京').first().expect();
        await terminal.getByText('Copy').first().expect();
        await terminal.getByText('PDF current').expect();
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
        await terminal.getByText('Saved TeX document.').expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), RESUME);
    });
});
