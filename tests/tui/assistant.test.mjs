import assert from 'node:assert/strict';
import { execFile, execFileSync } from 'node:child_process';
import { copyFile, mkdir, readFile, symlink, writeFile } from 'node:fs/promises';
import { join, delimiter } from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { promisify } from 'node:util';
import test from 'node:test';
import { ARTIFACTS, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

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
    await terminal.getByText('Reviewing AI changes').expect();
    await terminal.getByText('PDF proposed').expect();
    await prefix(terminal, 'a');
    await terminal.getByText('Accepted · PDF updated').expect();
    await prefix(terminal, 'w');
    await terminal.getByText(' · Saved').expect();
}

test('AI proposals refine the pending draft, protect the buffer, accept, save, and discard', async () => {
    await withTexo('assistant-review', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'AI review');

        const document = await documentPath(workspace);
        const original = await readFile(document, 'utf8');

        await ask(terminal, 'first');
        await terminal.getByText('Reviewing AI changes').expect();
        await terminal.getByText('PDF proposed').expect();
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
        await terminal.getByText('Reviewing AI changes').expect();
        await terminal.getByText('PDF proposed').expect();
        await prefix(terminal, 'd');
        await terminal.getByText('Proposal discarded.').expect();
        await prefix(terminal, 'w');
        await terminal.getByText(' · Saved').expect();
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
        await terminal.getByText(' · Saved').expect();
        const source = await readFile(await documentPath(workspace), 'utf8');
        assert.match(source, /Local edit wins/);
        assert.doesNotMatch(source, /First draft/);
    }, { prepare });
});

test('assistant copies unsaved text before proposing an edit to the open resume', async () => {
    await withTexo('assistant-copy', async ({ terminal, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Original resume');

        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        const working = saved + '\n% Unsaved editor text\n';
        await terminal.press('Ctrl+a');
        await paste(terminal, working);

        await ask(terminal, 'copy');
        await terminal.getByText('Assistant copy').first().expect();
        await terminal.getByText('PDF proposed').expect();

        const copyPath = join(workspace, 'assistant-copy', 'main.tex');
        assert.equal(await readFile(copyPath, 'utf8'), working);
        assert.equal(await readFile(document, 'utf8'), saved);

        await acceptAndSave(terminal);
        assert.match(await readFile(document, 'utf8'), /Draft after copying/);
        assert.equal(await readFile(copyPath, 'utf8'), working);
    }, { prepare });
});

test('assistant opens and compiles the first resume for review in one turn', async () => {
    await withTexo('assistant-bootstrap', async ({ terminal, workspace }) => {
        await terminal.resize(160, 50);
        await ask(terminal, 'bootstrap');

        await terminal.getByText('The PDF is ready for review.').expect();
        const document = await documentPath(workspace);
        assert.match(await readFile(document, 'utf8'), /My first shared resume/);

        await terminal.getByText('My first shared resume').expect();
        await terminal.getByText('PDF current').expect();
    }, { prepare });
});

test('a saved resume reaches preview even when the provider disconnects', async () => {
    await withTexo('assistant-create-disconnect', async ({ terminal }) => {
        await terminal.resize(160, 50);
        await ask(terminal, 'create then disconnect');

        await terminal.getByText('The PDF is ready for review.').expect();
        await terminal.getByText('Codex could not complete').expect();
        await terminal.getByText('PDF current').expect();
    }, { prepare });
});

test('the new resume carries the question and sends it with the next answer', async () => {
    await withTexo('assistant-create-clarification', async ({ terminal, root }) => {
        await terminal.resize(160, 50);
        await ask(terminal, 'create and ask');

        const question = 'Which city should appear on the resume?';
        await terminal.getByText('The PDF is ready for review.').expect();
        await terminal.getByText(question).expect();
        await terminal.getByText('PDF current').expect();
        await terminal.screenshot(join(ARTIFACTS, 'created-resume-question.png'));

        await ask(terminal, 'Alder City');
        await terminal.getByText('Fixture completed Alder City').expect();
        const context = JSON.parse(await readFile(join(root, 'bin/context.json'), 'utf8'));
        assert.match(context.current_tex, /My first shared resume/);

        const conversation = context.conversation.filter(message =>
            ['create and ask', question, 'Alder City'].includes(message.content)
        );
        assert.deepEqual(conversation, [
            { role: 'user', content: 'create and ask' },
            { role: 'assistant', content: question },
            { role: 'user', content: 'Alder City' },
        ]);
    }, { prepare });
});

test('a newly created resume with invalid TeX opens with diagnostics', async () => {
    await withTexo('assistant-create-failure', async ({ terminal, workspace }) => {
        await terminal.resize(160, 50);
        await ask(terminal, 'broken-create');

        await terminal.getByText('Resume saved. PDF build failed.').expect();
        await terminal.getByText('ready for review').expect({ not: true });

        const document = await documentPath(workspace);
        assert.match(await readFile(document, 'utf8'), /unknownResumeCommand/);
        await assert.rejects(readFile(join(workspace, 'broken-resume', 'main.pdf')), { code: 'ENOENT' });
    }, { prepare });
});

test('creating a resume preserves a dirty editor and its saved source', async () => {
    await withTexo('assistant-create-dirty', async ({ terminal, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Original');

        const original = await documentPath(workspace);
        const saved = await readFile(original, 'utf8');
        await terminal.press('Ctrl+End');
        await paste(terminal, '\n% Unsaved inventory work');

        await ask(terminal, 'broken-create');
        await terminal.getByText('Kept your current view open').expect();
        await terminal.getByText('Unsaved inventory work').first().expect();
        assert.equal(await readFile(original, 'utf8'), saved);
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
        await terminal.getByText('Reviewing AI changes').expect({ not: true });
        await terminal.getByText('First draft').expect({ not: true });
    }, { prepare });
});

test('failed and cancelled refinements retain the last reviewable draft', async () => {
    await withTexo('assistant-refinement-recovery', async ({ terminal, root, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Refinement recovery');

        await ask(terminal, 'first');
        await terminal.getByText('Reviewing AI changes').expect();
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
        await terminal.getByText('Reviewing AI changes').expect();
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
        await terminal.getByText('Reviewing AI changes').expect();
        await terminal.getByText('texo / chat').expect();
        await terminal.getByText('[ Proposed ]').expect();
        await prefix(terminal, 'z');
        await terminal.getByText('Reviewing AI changes').expect({ not: true });
        await prefix(terminal, 'd');
        await terminal.getByText('PDF current').expect();
    }, { prepare });
});

test('quit confirmation preserves a pending AI proposal and accepted unsaved edits', async () => {
    await withTexo('quit-ai-proposal', async ({ terminal, workspace }) => {
        await terminal.resize(160, 50);
        await createResume(terminal, 'Protected AI work');
        const document = await documentPath(workspace);
        const saved = await readFile(document, 'utf8');
        await ask(terminal, 'first');
        await terminal.getByText('Reviewing AI changes').expect();
        await terminal.getByText('PDF proposed').expect();

        await terminal.press('Ctrl+q');
        await terminal.getByText('Quit Texo?').expect();
        await terminal.getByText('pending AI work or proposals').expect();
        await terminal.press('Esc');
        await terminal.getByText('Quit Texo?').expect({ not: true });
        await terminal.getByText('Reviewing AI changes').expect();
        assert.equal(await readFile(document, 'utf8'), saved);

        await prefix(terminal, 'a');
        await terminal.getByText('Accepted · PDF updated').expect();
        await terminal.press('Ctrl+q');
        await terminal.getByText('unsaved text').expect();
        await terminal.press('Esc');
        await prefix(terminal, 'w');
        await terminal.getByText(' · Saved').expect();
        assert.match(await readFile(document, 'utf8'), /First draft/);
    }, { prepare });
});

test('model command opens the picker and persists the selected model', async () => {
    await withTexo('assistant-model-command', async ({ terminal, root }) => {
        await createResume(terminal, 'Model selection');
        await ask(terminal, '/model');
        await terminal.getByText('Choose a model').expect();

        await terminal.press('Down', 'Enter');
        await terminal.getByText('Choose a model').expect({ not: true });
        await terminal.getByText('Model selected:').expect();

        const profilePath = join(root, 'config/texo/provider.json');
        const profile = JSON.parse(await readFile(profilePath, 'utf8'));
        assert.equal(profile.provider, 'codex');
        assert.ok(profile.model, 'The chosen model must be saved');
        await terminal.getByText(profile.model).first().expect();

        await terminal.getByText('Ask Texo...').click();
        await paste(terminal, '/model');
        await terminal.press('Enter');
        await terminal.getByText('Choose a model').expect();
        await terminal.getByText(`› ${profile.model}`).expect();
        await terminal.press('Escape');
    }, { prepare });
});

test('assistant Markdown is styled while the prompt and provider history remain literal', async () => {
    await withTexo('assistant-markdown', async ({ terminal, root }) => {
        await createResume(terminal, 'License example');
        await ask(terminal, '**markdown**');
        await terminal.getByText('SPDX-License-Identifier: MIT').expect();
        await prefix(terminal, 'z');
        await terminal.resize(80, 28);

        const bold = terminal.getByStyle({ bold: true });
        const italic = terminal.getByStyle({ italic: true });
        const link = terminal.getByStyle({ underlineStyle: 'single' });
        await terminal.getByText('License check').and(bold).expect();
        await terminal.getByText('MIT is a permissive license.').expect();
        await terminal.getByText('MIT').and(bold).expect();
        await terminal.getByText('permissive').and(italic).expect();
        await terminal.getByText('Open Source Initiative license text').and(link).expect();
        await terminal.getByText('https://').and(link).expect();
        await terminal.getByText('opensource.org/license/mit').and(link).expect();
        const code = terminal.getByStyle({ background: '#181825' });
        await terminal.getByText('LICENSE').and(code).expect();
        await terminal.getByText('Copyright 2026 Example Authors').and(code).expect();
        await terminal.getByText('**markdown**', { whitespace: 'exact' }).expect();

        await ask(terminal, 'advice');
        await terminal.getByText('Fixture completed advice').expect();
        const context = JSON.parse(await readFile(join(root, 'bin/context.json'), 'utf8'));
        const assistantReply = context.conversation.find(message => message.role === 'assistant');
        assert.match(assistantReply.content, /\*\*MIT\*\* is a \*permissive\*/);
        assert.ok(assistantReply.content.includes(
            '[Open Source Initiative license text](https://opensource.org/license/mit)',
        ));
        assert.equal(context.conversation[0].content, '**markdown**');
    }, { prepare, env: { NO_COLOR: '' } });
});

test('assistant Markdown copies across a wrapped link to the OS clipboard', {
    skip: process.env.TEXO_CLIPBOARD_GUARDED !== '1' ? 'Run with the OS pasteboard guard' : false,
}, async () => {
    await withTexo('assistant-markdown-clipboard', async ({ terminal }) => {
        await createResume(terminal, 'Clipboard example');
        await ask(terminal, '**markdown**');
        await terminal.getByText('SPDX-License-Identifier: MIT').expect();
        await prefix(terminal, 'z');
        await terminal.resize(46, 25);

        const linkLabel = terminal.getByText('Open Source');
        const linkSuffix = terminal.getByText('opensource.org/license/mit');
        await linkLabel.expect();
        await linkSuffix.expect();

        const label = await linkLabel.location();
        const url = await linkSuffix.location();
        assert.ok(url.start.row > label.start.row);

        const draft = 'Draft stays here';
        await paste(terminal, draft);
        await terminal.getByText(draft).expect();
        execFileSync('pbcopy', { input: 'Clipboard seed' });

        await terminal.mouse.drag(label.start.column, label.start.row, url.end.column, url.end.row);
        const selected = terminal.getByStyle({ inverse: true, underlineStyle: 'single' });
        await terminal.getByText('Open Source').and(selected).expect();
        await terminal.getByText('https://').and(selected).expect();
        await terminal.getByText('opensource.org/license/mit').and(selected).expect();
        await terminal.press('Ctrl+c');
        await terminal.getByText('Copied chat selection').expect();

        const copied = execFileSync('pbpaste', { encoding: 'utf8' });
        assert.equal(copied, 'Open Source Initiative license text (https://opensource.org/license/mit');
        await terminal.getByText(draft).expect();
    }, { prepare, env: { NO_COLOR: '' } });
});
