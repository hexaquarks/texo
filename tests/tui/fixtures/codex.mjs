#!/usr/bin/env node
// Script only the provider boundary; Texo still runs its real tools and compiler.
import assert from 'node:assert/strict';
import { access, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const WAIT_TIMEOUT_MS = 20_000;
const POLL_INTERVAL_MS = 20;
const MARKDOWN_PROMPT = '**markdown**';
const SCENARIOS = new Set([
    'first', 'refine', 'repair', 'advice', 'hold', 'fail', 'copy', 'bootstrap', 'broken-create', MARKDOWN_PROMPT,
]);
const FIRST_RESUME = '\\documentclass{article}\n\\begin{document}\nMy first shared resume.\n\\end{document}\n';
const MARKDOWN_REPLY = [
    '## License check',
    '',
    '**MIT** is a *permissive* license.',
    '',
    '- Read the [Open Source Initiative license text](https://opensource.org/license/mit).',
    '- Keep a copy of `LICENSE`.',
    '',
    '```text',
    '  Copyright 2026 Example Authors',
    '  SPDX-License-Identifier: MIT',
    '```',
].join('\n');
const controls = dirname(fileURLToPath(import.meta.url));

async function readRequest() {
    process.stdin.setEncoding('utf8');
    let input = '';
    for await (const chunk of process.stdin) {
        input += chunk;
    }

    const objects = input
        .split('\n')
        .filter(line => line.startsWith('{'))
        .map(line => JSON.parse(line));
    const context = objects.find(object => 'current_tex' in object);
    const protocol = objects.find(object => 'tool_history' in object);
    assert.ok(context && protocol, 'Expected Texo context and tool history');

    const prompt = context.conversation.findLast(message => message.role === 'user')?.content;
    assert.ok(SCENARIOS.has(prompt), `Unknown scripted scenario: ${prompt}`);
    await writeFile(join(controls, 'context.json'), JSON.stringify(context));
    return { prompt, history: protocol.tool_history, tools: protocol.tools };
}

async function holdReply() {
    await writeFile(join(controls, 'started'), '');
    const deadline = Date.now() + WAIT_TIMEOUT_MS;

    while (Date.now() < deadline) {
        try {
            await access(join(controls, 'release'));
            return;
        } catch (error) {
            if (error.code !== 'ENOENT') {
                throw error;
            }
        }
        await setTimeout(POLL_INTERVAL_MS);
    }
    throw new Error('Test did not release the held reply');
}

function editAndCompile(content, identifier) {
    const source = `\\documentclass{article}\n\\begin{document}\n${content}\n\\end{document}\n`;
    return [
        {
            id: `${identifier}-edit`,
            name: 'propose_edit',
            arguments_json: JSON.stringify({ tex: source }),
        },
        {
            id: `${identifier}-build`,
            name: 'compile_resume',
            arguments_json: '{}',
        },
    ];
}

function copyAndEdit(history) {
    if (history.length === 0) {
        return [
            {
                id: 'save-copy',
                name: 'create_resume',
                arguments_json: JSON.stringify({
                    name: 'Assistant copy',
                    source_document_id: 'current',
                    tex: null,
                }),
            },
        ];
    }

    const [copy] = history[0].results;
    assert.equal(copy.is_error, false, JSON.stringify(copy.output));
    assert.equal(copy.output.saved, true);

    if (history.length === 1) {
        return editAndCompile('Draft after copying', 'after-copy');
    }

    return [];
}

function createFirstResume(history, tools) {
    assert.ok(tools.some(tool => tool.name === 'create_resume'));
    assert.ok(!tools.some(tool => tool.name === 'propose_edit'));

    if (history.length === 0) {
        return [
            {
                id: 'list-workspace',
                name: 'list_resumes',
                arguments_json: '{}',
            },
        ];
    }

    if (history.length === 1) {
        const [list] = history[0].results;
        assert.equal(list.is_error, false);
        assert.deepEqual(list.output.documents, []);
        return [
            {
                id: 'create-first',
                name: 'create_resume',
                arguments_json: JSON.stringify({
                    name: 'First shared resume',
                    source_document_id: null,
                    tex: FIRST_RESUME,
                }),
            },
        ];
    }

    if (history.length === 2) {
        const [creation] = history[1].results;
        assert.equal(creation.is_error, false);
        assert.equal(creation.output.saved, true);
        return [
            {
                id: 'read-created',
                name: 'read_resume',
                arguments_json: JSON.stringify({ document_id: creation.output.created.id }),
            },
        ];
    }

    const [read] = history[2].results;
    assert.equal(read.is_error, false);
    assert.equal(read.output.tex, FIRST_RESUME);
    return [];
}

async function respond() {
    const args = process.argv.slice(2);
    if (args.length === 2 && args[0] === 'login' && args[1] === 'status') {
        return;
    }

    const { prompt, history, tools } = await readRequest();
    if (prompt === 'fail') {
        process.exitCode = 1;
        return;
    }
    if (prompt === 'hold' && history.length === 0) {
        await holdReply();
    }

    let calls = [];
    const isAdvice = prompt === 'advice' || prompt === MARKDOWN_PROMPT;
    if (prompt === 'copy') {
        calls = copyAndEdit(history);
    } else if (prompt === 'broken-create' && history.length === 0) {
        calls = [{
            id: 'broken-resume',
            name: 'create_resume',
            arguments_json: JSON.stringify({
                name: 'Broken resume',
                tex: FIRST_RESUME.replace('My first shared resume.', '\\unknownResumeCommand'),
                source_document_id: null,
            }),
        }];
    } else if (prompt === 'bootstrap') {
        calls = createFirstResume(history, tools);
    } else if (!isAdvice && history.length === 0) {
        let content = 'First draft';
        if (prompt === 'refine') {
            content = 'Refined draft';
        } else if (prompt === 'repair') {
            content = '\\undefinedTestCommand';
        }
        calls = editAndCompile(content, 'initial');
    } else if (prompt === 'repair' && history.length === 1) {
        calls = editAndCompile('Repaired draft', 'repair');
    }

    const outputFlag = args.indexOf('-o');
    assert.ok(outputFlag >= 0 && args[outputFlag + 1], 'Expected a Codex output file');
    const reply = prompt === MARKDOWN_PROMPT ? MARKDOWN_REPLY : `Fixture completed ${prompt}`;
    const response = { reply, proposed_tex: null, tool_calls: calls };
    await writeFile(args[outputFlag + 1], JSON.stringify(response));
}

await respond();
