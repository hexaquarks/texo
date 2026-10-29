import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TuiTest } from '@microsoft/tui-test';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const BACKEND = process.env.TEXO_TUI_BACKEND ?? 'alacritty';
export const ARTIFACTS = join(REPOSITORY, 'target/tui-artifacts', BACKEND);
export const UI_TIMEOUT_MS = 15_000;

// tui-test exposes raw writes but no paste operation. These xterm bracketed-paste
// delimiters make multiline text one paste event rather than editor key commands.
const PASTE_START = '\x1b[200~';
const PASTE_END = '\x1b[201~';
export const RESUME = '\\documentclass{article}\n\\begin{document}\nTerminal workflow resume.\n\\end{document}\n';

export async function withTexo(name, scenario, options = {}) {
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
        const preparedEnvironment = options.prepare ? await options.prepare(root, workspace) : {};
        const restart = async ({ configuredWorkspace = false } = {}) => {
            await terminal.closeQuiet();
            await terminal.run(join(REPOSITORY, 'target/debug/texo'), configuredWorkspace ? [] : [workspace], {
                cols: 120,
                rows: 35,
                env: {
                    XDG_CONFIG_HOME: join(root, 'config'),
                    XDG_STATE_HOME: join(root, 'state'),
                    TERM: 'xterm-256color',
                    ...preparedEnvironment,
                    ...options.env,
                },
            });
        };
        await restart();
        await terminal.getByText('No resumes yet').expect();
        await scenario({ terminal, root, workspace, restart });
    } catch (error) {
        await writeFile(join(ARTIFACTS, `${name}-failure.txt`), await terminal.text());
        await terminal.screenshot(join(ARTIFACTS, `${name}-failure.png`));
        throw error;
    } finally {
        await terminal.closeQuiet();
        await rm(root, { recursive: true, force: true });
    }
}

export async function paste(terminal, text) {
    await terminal.write(`${PASTE_START}${text}${PASTE_END}`);
}

export async function prefix(terminal, key) {
    await terminal.press('Ctrl+x', key);
}

export async function createResume(terminal, name) {
    await prefix(terminal, 'n');
    await terminal.getByText('New resume').first().expect();
    await paste(terminal, name);
    await terminal.press('Enter');
    await terminal.getByText('PDF current').expect();
    await terminal.getByText(name).first().expect();
}

export async function documentPath(workspace) {
    const entries = await readdir(workspace, { withFileTypes: true });
    const project = entries.find(entry => entry.isDirectory());
    assert.ok(project, 'Creating a resume must create a project directory');
    return join(workspace, project.name, 'main.tex');
}

export async function action(terminal, name) {
    await prefix(terminal, 'p');
    await terminal.getByText(name).expect();
    await terminal.getByText(name).click();
}

export async function replaceField(terminal, value) {
    await terminal.press('Home', 'Shift+End');
    await paste(terminal, value);
}
