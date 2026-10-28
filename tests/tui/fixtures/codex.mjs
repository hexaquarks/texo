#!/usr/bin/env node
// Script only the provider boundary; Texo still runs its real tools and compiler.
import assert from 'node:assert/strict';
import { access, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const WAIT_TIMEOUT_MS = 20_000;
const POLL_INTERVAL_MS = 20;
const SCENARIOS = new Set(['first', 'refine', 'repair', 'advice', 'hold', 'fail']);
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
    return { prompt, history: protocol.tool_history };
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

async function respond() {
    const args = process.argv.slice(2);
    if (args.length === 2 && args[0] === 'login' && args[1] === 'status') {
        return;
    }

    const { prompt, history } = await readRequest();
    if (prompt === 'fail') {
        process.exitCode = 1;
        return;
    }
    if (prompt === 'hold' && history.length === 0) {
        await holdReply();
    }

    let calls = [];
    if (prompt !== 'advice' && history.length === 0) {
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
    const response = { reply: `Fixture completed ${prompt}`, proposed_tex: null, tool_calls: calls };
    await writeFile(args[outputFlag + 1], JSON.stringify(response));
}

await respond();
