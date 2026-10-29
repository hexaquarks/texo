import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { copyFile, mkdir, readFile, symlink, writeFile } from 'node:fs/promises';
import { join, delimiter } from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { promisify } from 'node:util';
import test from 'node:test';
import { action, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

const run = promisify(execFile);
const POLL_INTERVAL_MS = 20;
const FIXTURE_TIMEOUT_MS = 10_000;

async function prepare(root) {
    const bin = join(root, 'bin');
    await mkdir(bin);

    const executable = join(bin, 'codex.mjs');
    await copyFile(new URL('./fixtures/codex.mjs', import.meta.url), executable);
    await run('chmod', ['u+x', executable]);
    await symlink(executable, join(bin, 'codex'));

    return { PATH: bin + delimiter + process.env.PATH };
}

async function ask(terminal, prompt) {
    // Refuse to send anything if the user's configured provider would bypass our fixture.
    await terminal.getByText('Codex ·').expect();
    await terminal.getByText('Ask Texo...').click();
    await paste(terminal, prompt);
    await terminal.press('Enter');
}

async function waitStarted(root) {
    const deadline = Date.now() + FIXTURE_TIMEOUT_MS;
    while (Date.now() < deadline) {
        try {
            await readFile(join(root, 'bin/started'));
            return;
        } catch (error) {
            if (error.code !== 'ENOENT') {
                throw error;
            }
        }
        await setTimeout(POLL_INTERVAL_MS);
    }
    assert.fail('The scripted Codex request never started');
}

async function acceptAndSave(terminal) {
    await terminal.getByText('Unapproved changes').expect();
    await terminal.getByText('PDF current').expect();
    await prefix(terminal, 'a');
    await terminal.getByText('Accepted · PDF updated').expect();
    await prefix(terminal, 'w');
    await terminal.getByText('Saved TeX document.').expect();
}

test('AI proposals refine the pending draft, protect the buffer, accept, save, and discard', async () => {
    await withTexo('assistant-review', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'AI review');

        const document = await documentPath(workspace);
        const original = await readFile(document, 'utf8');

        await ask(terminal, 'first');
        await terminal.getByText('Unapproved changes').expect();
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('First draft').first().click();
        await paste(terminal, 'BLOCKED EDIT');
        assert.equal(await readFile(document, 'utf8'), original);
        await prefix(terminal, 'm');

        await ask(terminal, 'refine');
        await terminal.getByText('Fixture completed refine').expect();
        const context = JSON.parse(await readFile(join(root, 'bin/context.json'), 'utf8'));
        assert.match(context.current_tex, /First draft/);

        await acceptAndSave(terminal);

        const accepted = await readFile(document, 'utf8');
        assert.match(accepted, /Refined draft/);
        assert.doesNotMatch(accepted, /BLOCKED EDIT/);

        await ask(terminal, 'first');
        await terminal.getByText('Unapproved changes').expect();
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'd');
        await terminal.getByText('Proposal discarded.').expect();
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        assert.equal(await readFile(document, 'utf8'), accepted);

        await ask(terminal, 'advice');
        await terminal.getByText('Assistant replied without changing').expect();
        assert.equal(await readFile(document, 'utf8'), accepted);
    }, { prepare });
});

test('late AI replies preserve newer local edits across a resize', async () => {
    await withTexo('assistant-stale', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Stale reply');

        await ask(terminal, 'hold');
        await waitStarted(root);
        await terminal.getByText('\\documentclass').first().click();
        await terminal.press('Ctrl+End');
        await paste(terminal, '\n% Local edit wins');
        await terminal.resize(60, 16);
        await terminal.resize(120, 35);
        await writeFile(join(root, 'bin/release'), '');
        await terminal.getByText('Proposal is stale').expect();
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        const source = await readFile(await documentPath(workspace), 'utf8');
        assert.match(source, /Local edit wins/);
        assert.doesNotMatch(source, /First draft/);
    }, { prepare });
});

test('AI cancellation, provider failure, and failed compilation all allow another request', async () => {
    await withTexo('assistant-recovery', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'AI recovery');

        const document = await documentPath(workspace);
        const original = await readFile(document, 'utf8');

        await ask(terminal, 'hold');
        await waitStarted(root);
        await terminal.press('Escape');
        await terminal.getByText('Request cancelled.').expect();

        await ask(terminal, 'fail');
        await terminal.getByText('Assistant failed.').expect();
        assert.equal(await readFile(document, 'utf8'), original);

        await ask(terminal, 'repair');

        await acceptAndSave(terminal);
        assert.match(await readFile(document, 'utf8'), /Repaired draft/);
        await action(terminal, 'Clear conversation');
        await terminal.getByText('Fixture completed repair').expect({ not: true });
    }, { prepare });
});

test('late AI work stays with its original resume when the user switches projects', async () => {
    await withTexo('assistant-project-switch', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Original project');
        const first = await documentPath(workspace);

        await ask(terminal, 'hold');
        await waitStarted(root);
        await createResume(terminal, 'Other project');
        await writeFile(join(root, 'bin/release'), '');
        await terminal.getByText('Original project').first().click();

        await acceptAndSave(terminal);
        assert.match(await readFile(first, 'utf8'), /First draft/);
        await terminal.getByText('Other project').first().click();
        await terminal.getByText('PDF current').expect();
        await terminal.getByText('Unapproved changes').expect({ not: true });
        await terminal.getByText('First draft').expect({ not: true });
    }, { prepare });
});

test('failed and cancelled refinements retain the last reviewable draft', async () => {
    await withTexo('assistant-refinement-recovery', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Refinement recovery');

        await ask(terminal, 'first');
        await terminal.getByText('Unapproved changes').expect();
        await prefix(terminal, 'm');

        await ask(terminal, 'fail');
        await terminal.getByText('Codex could not complete').expect();

        await ask(terminal, 'hold');
        await waitStarted(root);
        await terminal.press('Escape');
        await terminal.getByText('Request cancelled.').expect();

        await acceptAndSave(terminal);
        assert.match(await readFile(await documentPath(workspace), 'utf8'), /First draft/);
    }, { prepare });
});

test('a proposal arriving during export prevents publishing and explains why', async () => {
    await withTexo('assistant-export-race', async ({ terminal, root }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Export race');

        await ask(terminal, 'hold');
        await waitStarted(root);
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').expect();
        const destination = join(root, 'application.pdf');
        await terminal.press('Home', 'Shift+End');
        await paste(terminal, destination);
        await writeFile(join(root, 'bin/release'), '');
        await terminal.getByText('Unapproved changes').expect();
        await terminal.press('Enter');
        await terminal.getByText('Export PDF').expect({ not: true });
        await assert.rejects(readFile(destination), { code: 'ENOENT' });
        await terminal.getByText('PDF changed while exporting').expect();
    }, { prepare });
});

test('login provider and key screens cancel without changing the configured connection', async () => {
    await withTexo('assistant-login-cancel', async ({ terminal }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Login cancellation');

        await ask(terminal, '/login');
        await terminal.getByText('Connect an assistant').expect();
        await terminal.getByText('Anthropic / Claude').click();
        await terminal.getByText('Select a sign-in method').expect();
        await terminal.getByText('Sign in with a new API key').click();
        await terminal.getByText('Enter your API key').expect();
        const fakeKey = 'synthetic-key-never-submitted';
        await paste(terminal, fakeKey);
        await terminal.getByText('••••').expect();
        await terminal.getByText(fakeKey).expect({ not: true });
        await terminal.resize(40, 12);
        await terminal.press('Escape');
        await terminal.resize(160, 50);
        await terminal.getByText('Codex ·').expect();
        // Wait for the expanded layout before locating the composer after resize.
        await terminal.getByText('Ask Texo to refine the open resume.').expect();

        await ask(terminal, 'advice');
        await terminal.getByText('Assistant replied without changing').expect();
    }, { prepare });
});

test('a proposal restores adjacent panes after a workspace-expanded chat request', async () => {
    await withTexo('expanded-chat-proposal', async ({ terminal }) => {
        await createResume(terminal, 'Expanded review');
        await terminal.getByText('Ask Texo...').click();
        await prefix(terminal, 'u');
        await prefix(terminal, 'z');
        await ask(terminal, 'first');
        await terminal.getByText('Unapproved changes').expect();
        await terminal.getByText('texo / chat').expect();
        await terminal.getByText('[ Proposed ]').expect();
        await prefix(terminal, 'z');
        await terminal.getByText('Unapproved changes').expect({ not: true });
        await prefix(terminal, 'd');
        await terminal.getByText('PDF current').expect();
    }, { prepare });
});
