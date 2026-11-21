import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';

import { ARTIFACTS, paste, prefix, UI_TIMEOUT_MS, withTexo } from './helpers.mjs';

const executeFile = promisify(execFile);
const STARTERS = [
    { template: 'Blank', sample: 'Your resume starts here.' },
    { template: "Jake's Resume", sample: 'Avery Morgan' },
    { template: 'Rezume', sample: 'Jane Doe' },
    { template: 'AustereCV', sample: 'John Smith' },
    { template: 'Software Engineering', sample: 'Dwight Schrute' },
    { template: 'Modern Simple CV', sample: 'Digital Marketing Strategist', pages: 2 },
    { template: 'Minimal CV Template', sample: 'YOUR NAME' },
    { template: 'Libre CV', sample: 'John Doe' },
    { template: "Cole's Resume", sample: 'JOHN DOE' },
    { template: 'Academic CV by Diego', sample: 'Curriculum Vitae', pages: 6 },
    { template: 'Resume by Gabriel', sample: 'John Zlad Doe' },
    { template: 'Minimal LaTeX CV', sample: 'John Doe' },
    { template: 'LaTeX Resume Template', sample: 'Jane Coder' },
];

function previewLabel(template) {
    const index = STARTERS.findIndex(starter => starter.template === template);
    assert.ok(index >= 0, `Unknown template ${template}`);
    return `PDF preview · ${index + 1}/${STARTERS.length}`;
}

test('every gallery starter opens as editable source and compiles to PDF', async () => {
    await withTexo('gallery-create', async ({ terminal, workspace }) => {
        for (const [index, starter] of STARTERS.entries()) {
            const existingProjects = await readdir(workspace);
            await prefix(terminal, 'n');
            await terminal.getByText('New resume · Templates').expect();

            for (let step = 0; step < index; step += 1) {
                await terminal.press('Down');
            }
            await terminal.getByText(previewLabel(starter.template)).expect();
            await terminal.getByText('▀').first().expect();
            await terminal.press('Enter');
            await terminal.getByText(`Template · ${starter.template}`).expect();

            const name = `Gallery draft ${index + 1}`;
            await paste(terminal, name);
            await terminal.press('Enter');
            await terminal.getByText('Resume name').expect({ not: true });
            await terminal.getByText(name).first().expect();
            await terminal.getByText('PDF current').expect();

            const projectDirectories = await readdir(workspace);
            const newProjects = projectDirectories.filter(project => !existingProjects.includes(project));
            assert.equal(newProjects.length, 1);

            const [project] = newProjects;
            const source = await readFile(join(workspace, project, 'main.tex'), 'utf8');
            assert.ok(source.toLowerCase().includes(starter.sample.toLowerCase()));

            if (starter.template === "Jake's Resume") {
                assert.ok(source.includes('Copyright (c) 2020 Jake Gutierrez'));
                assert.ok(source.includes('THE SOFTWARE IS PROVIDED "AS IS"'));
            }

            const pdf = join(workspace, project, 'main.pdf');
            const { stdout } = await executeFile('pdftotext', [pdf, '-'], {
                timeout: UI_TIMEOUT_MS,
            });
            const pages = stdout.split('\f').filter(page => page.trim());
            assert.equal(pages.length, starter.pages ?? 1, `${starter.template} page count`);
            assert.ok(
                stdout.toLowerCase().includes(starter.sample.toLowerCase()),
                `PDF must contain ${starter.sample}`,
            );

            if (starter.template === 'Academic CV by Diego') {
                assert.match(pages[3], /^\s*Thesis\b/m);
                assert.match(pages[4], /^\s*Volunteering\b/im);
            }
        }
    }, { env: { NO_COLOR: '' } });
});

test('gallery preserves the selection and draft name when returning from naming', async () => {
    await withTexo('gallery-browse', async ({ terminal, workspace }) => {
        await prefix(terminal, 'n');
        await terminal.getByText('Start from scratch').expect();
        await terminal.getByText(previewLabel('Blank')).expect();
        await terminal.press('Down');
        await terminal.getByText(previewLabel("Jake's Resume")).expect();
        await terminal.press('Up');
        await terminal.getByText(previewLabel('Blank')).expect();

        await terminal.press('End');
        await terminal.getByText(previewLabel('LaTeX Resume Template')).expect();
        await terminal.resize(60, 18);
        await terminal.getByText('Software resume').expect();
        await terminal.screenshot(join(ARTIFACTS, 'template-gallery-compact.png'));

        await terminal.resize(120, 35);
        await terminal.getByText('Rezume').click();
        await terminal.getByText(previewLabel('Rezume')).expect();
        await terminal.getByText('Start from scratch').click();
        await terminal.getByText(previewLabel('Rezume')).expect();
        await terminal.screenshot(join(ARTIFACTS, 'template-gallery.png'));
        await terminal.press('Enter');
        await terminal.getByText('Resume name').expect();
        await paste(terminal, 'Design portfolio');

        await terminal.press('Escape');
        await terminal.getByText(previewLabel('Rezume')).expect();
        await terminal.press('Enter');
        await terminal.getByText('Design portfolio').expect();

        await terminal.press('Escape');
        await terminal.getByText('New resume · Templates').expect();
        await terminal.press('Escape');
        await terminal.getByText('No resumes yet').expect();
        assert.deepEqual(await readdir(workspace), []);
    }, { env: { NO_COLOR: '' } });
});
