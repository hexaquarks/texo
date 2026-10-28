import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import test from 'node:test';
import { TuiTest } from '@microsoft/tui-test';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const BACKEND = process.env.TEXO_TUI_BACKEND ?? 'alacritty';
const ARTIFACTS = join(REPOSITORY, 'target/tui-artifacts', BACKEND);
const UI_TIMEOUT_MS = 15_000;
const PDF_INSPECTION_TIMEOUT_MS = 10_000;
const executeFile = promisify(execFile);

// tui-test exposes raw writes but no paste operation. These xterm bracketed-paste
// delimiters make multiline text one paste event rather than editor key commands.
const PASTE_START = '\x1b[200~';
const PASTE_END = '\x1b[201~';
const RESUME = '\\documentclass{article}\n\\begin{document}\nTerminal workflow resume.\n\\end{document}\n';

async function withTexo(name, scenario) {
    const root = await mkdtemp(join(tmpdir(), 'texo-terminal-'));
    const workspace = join(root, 'resumes');
    await mkdir(workspace);
    await mkdir(ARTIFACTS, { recursive: true });

    const terminal = TuiTest.ephemeral(name, {
        backend: BACKEND,
        timeouts: { text: UI_TIMEOUT_MS },
        recording: { directory: ARTIFACTS },
        artifacts: { dir: ARTIFACTS, onFailure: 'all' },
    });

    try {
        await terminal.run(join(REPOSITORY, 'target/debug/texo'), [workspace], {
            cols: 120,
            rows: 35,
            env: { XDG_CONFIG_HOME: join(root, 'config'), TERM: 'xterm-256color' },
        });
        await terminal.getByText('No resumes yet').expect();
        await scenario({ terminal, root, workspace });
    } catch (error) {
        await writeFile(join(ARTIFACTS, `${name}-failure.txt`), await terminal.text());
        await terminal.screenshot(join(ARTIFACTS, `${name}-failure.png`));
        throw error;
    } finally {
        await terminal.closeQuiet();
        await rm(root, { recursive: true, force: true });
    }
}

async function paste(terminal, text) {
    await terminal.write(`${PASTE_START}${text}${PASTE_END}`);
}

async function prefix(terminal, key) {
    await terminal.press('Ctrl+x', key);
}

async function createResume(terminal, name) {
    await prefix(terminal, 'n');
    await terminal.getByText('New resume').first().expect();
    await paste(terminal, name);
    await terminal.press('Enter');
    await terminal.getByText('PDF current').expect();
    await terminal.getByText(name).first().expect();
}

async function documentPath(workspace) {
    const entries = await readdir(workspace, { withFileTypes: true });
    const project = entries.find(entry => entry.isDirectory());
    assert.ok(project, 'Creating a resume must create a project directory');
    return join(workspace, project.name, 'main.tex');
}

test('create, paste Unicode, save, build, and export through the terminal', async () => {
    await withTexo('edit-export', async ({ terminal, workspace, root }) => {
        await createResume(terminal, 'Résumé 東京');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME);
        await prefix(terminal, 'w');
        await terminal.getByText('Saved TeX document.').expect();
        assert.equal(await readFile(await documentPath(workspace), 'utf8'), RESUME);

        await prefix(terminal, 'b');
        await terminal.getByText('PDF current').expect();
        await prefix(terminal, 'e');
        await terminal.getByText('Export PDF').expect();
        await terminal.press('Home', 'Shift+End');
        const exported = join(root, 'application.pdf');
        await paste(terminal, exported);
        await terminal.press('Enter');
        await terminal.getByText('Exported').expect();
        const { stdout } = await executeFile('pdftotext', [exported, '-'], {
            timeout: PDF_INSPECTION_TIMEOUT_MS,
        });
        assert.match(stdout, /Terminal workflow resume/);
        await terminal.screenshot(join(ARTIFACTS, 'edit-export.png'));
    });
});

test('resize down to one cell, restore, and click a scrolled action', async () => {
    await withTexo('resize-menu', async ({ terminal }) => {
        await createResume(terminal, 'Resize test');
        for (const [cols, rows] of [[80, 24], [40, 12], [20, 5], [1, 1], [120, 35]]) {
            await terminal.resize(cols, rows);
            await terminal.waitIdle();
        }
        await terminal.getByText('Resize test').first().expect();
        await terminal.resize(40, 12);
        await prefix(terminal, 'p');
        await terminal.getByText('New resume').expect();
        const settingsIndex = 10;
        for (let index = 0; index < settingsIndex; index += 1) {
            await terminal.press('Down');
        }
        await terminal.getByText('Settings').expect();
        await terminal.getByText('Settings').click();
        await terminal.getByText('Resume folder').expect();
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
        await terminal.getByText('Source .tex file').expect();
        await paste(terminal, source);
        await terminal.getByText('Imported resume').click();
        await paste(terminal, 'Mouse imported');
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
        await terminal.getByText('Source .tex file').expect();
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
        await terminal.getByText('Source .tex file').expect();
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
