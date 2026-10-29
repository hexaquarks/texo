import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

async function savedSource(terminal, workspace) {
    await prefix(terminal, 'w');
    await terminal.getByText('Saved TeX document.').expect();
    await terminal.getByText('unsaved').expect({ not: true });

    return readFile(await documentPath(workspace), 'utf8');
}

async function enableVim(terminal) {
    await prefix(terminal, 's');
    await terminal.getByText('Resume folder').expect();
    await terminal.press('Ctrl+v');
    await terminal.press('Enter');
    await terminal.getByText('NORMAL').expect();
}

test('standard editing handles undo, redo, literal search, and Unicode wrapping', async () => {
    await withTexo('standard-editor', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Editor');
        const source = 'alpha [literal] beta\nRésumé 李 東京\nthird line\n';
        await terminal.press('Ctrl+a');
        await paste(terminal, source);
        await paste(terminal, '% appended');
        await terminal.getByText('% appended').expect();
        await terminal.press('Ctrl+z');
        await terminal.getByText('% appended').expect({ not: true });
        assert.equal(await savedSource(terminal, workspace), source);
        await terminal.press('Ctrl+y');
        await terminal.getByText('% appended').expect();
        assert.equal(await savedSource(terminal, workspace), source + '% appended');

        await terminal.press('Ctrl+f');
        await terminal.getByText('Find:').expect();
        await paste(terminal, '[literal]');
        await terminal.press('Enter', 'Shift+Enter', 'Escape');
        await terminal.getByText('Find:').expect({ not: true });
        await terminal.resize(35, 15);
        await terminal.resize(120, 35);
        await terminal.getByText('Résumé 李 東京').expect();
        assert.equal(await savedSource(terminal, workspace), source + '% appended');
    });
});

test('mouse selection replaces only the selected editor text', async () => {
    await withTexo('editor-selection', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Selection');
        await terminal.press('Ctrl+a');
        await paste(terminal, 'alpha beta\nRésumé 李\n');
        await terminal.getByText('alpha beta').expect();
        const word = await terminal.getByText('alpha').location();
        await terminal.mouse.drag(word.start.column, word.start.row, word.end.column, word.end.row);
        await paste(terminal, 'FIRST');
        assert.equal(await savedSource(terminal, workspace), 'FIRST beta\nRésumé 李\n');
    });
});

test('Vim change objects, repeat, undo, redo, and visual deletion survive terminal input', async () => {
    await withTexo('vim-editing', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Vim');
        await terminal.press('Ctrl+a');
        await paste(terminal, 'one two three\nsecond line\nthird line\n');
        await enableVim(terminal);
        await terminal.press('g', 'g', 'c', 'i', 'w');
        await terminal.getByText('INSERT').expect();
        await paste(terminal, 'NEW');
        await terminal.press('Escape');
        await terminal.getByText('NORMAL').expect();
        await terminal.press('w', '.');
        await terminal.getByText('NEW NEW three').expect();
        assert.equal(await savedSource(terminal, workspace), 'NEW NEW three\nsecond line\nthird line\n');

        await terminal.press('u', 'u');
        await terminal.getByText('NEW two three').expect();
        assert.equal(await savedSource(terminal, workspace), 'NEW two three\nsecond line\nthird line\n');

        await terminal.press('Ctrl+r', 'Ctrl+r');
        await terminal.getByText('NEW NEW three').expect();
        assert.equal(await savedSource(terminal, workspace), 'NEW NEW three\nsecond line\nthird line\n');

        await terminal.press('g', 'g', 'j', 'V', 'd');
        await terminal.getByText('second line').expect({ not: true });
        assert.equal(await savedSource(terminal, workspace), 'NEW NEW three\nthird line\n');
    });
});

test('pending Vim operators cancel on dialogs and focus changes without consuming the next command', async () => {
    await withTexo('vim-focus', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Vim focus');
        const source = 'one two three\nsecond line\n';
        await terminal.press('Ctrl+a');
        await paste(terminal, source);
        await enableVim(terminal);
        await terminal.press('g', 'g', 'd');
        await prefix(terminal, 's');
        await terminal.getByText('Resume folder').expect();
        await terminal.press('Escape');
        await terminal.getByText('Resume folder').expect({ not: true });
        await terminal.press('w');
        assert.equal(await savedSource(terminal, workspace), source);

        await terminal.press('d');
        await terminal.getByText('Ask Texo...').click();
        await paste(terminal, 'Unsent draft');
        await terminal.getByText('Unsent draft').expect();
        await terminal.getByText('one two three').click();
        await terminal.press('w');
        assert.equal(await savedSource(terminal, workspace), source);
        await terminal.press('/');
        await paste(terminal, 'second');
        await terminal.press('Enter', 'n', 'N');
        assert.equal(await savedSource(terminal, workspace), source);
    });
});

test('OS clipboard copy, cut, and paste preserve Unicode text', {
    skip: process.env.TEXO_CLIPBOARD_GUARDED !== '1' ? 'Run test:clipboard to preserve the OS pasteboard' : false,
}, async () => {
    await withTexo('clipboard', async ({ terminal, workspace }) => {
        await createResume(terminal, 'Clipboard');
        const source = 'Clipboard Résumé 李\nsecond line\n';
        await terminal.press('Ctrl+a');
        await paste(terminal, source);
        await terminal.press('Ctrl+a', 'Ctrl+Insert');
        await terminal.getByText('Copied to').expect();
        await terminal.press('Ctrl+a', 'Shift+Delete');
        assert.equal(await savedSource(terminal, workspace), '');
        await terminal.press('Shift+Insert');
        await terminal.getByText('Clipboard Résumé 李').expect();
        assert.equal(await savedSource(terminal, workspace), source);
    });
});
