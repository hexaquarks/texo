import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import { writeFile } from 'node:fs/promises';
import test from 'node:test';
import { ARTIFACTS, RESUME, createResume, documentPath, paste, prefix, withTexo } from './helpers.mjs';

test('Problems explains a real build failure, navigates, shows details, and prepares an unsent fix', async () => {
    await withTexo('actionable-problems', async ({ terminal }) => {
        await terminal.resize(160, 45);
        await createResume(terminal, 'Errors demo');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME.replace('Terminal workflow resume.', '\\undefinedTestCommand'));
        await prefix(terminal, 'b');
        await terminal.getByText('Undefined control sequence').expect();
        await terminal.getByText('showing previous build').expect();
        await terminal.getByText('main.tex:3').expect();
        await terminal.screenshot(join(ARTIFACTS, 'actionable-problems.png'));

        await terminal.getByText('E Details').click();
        await terminal.getByText('sandbox-exec failed').expect();
        await terminal.getByText('Error details').expect();
        await terminal.press('End', 'PageUp', 'Home');
        await terminal.screenshot(join(ARTIFACTS, 'error-details-modal.png'));
        await terminal.press('Escape');
        await terminal.resize(70, 35);
        // Wait for the application to redraw, not just the terminal to reflow old cells.
        await terminal.getByText('Go to line   F Ask AI   E Details   D Hide').expect();
        await terminal.getByText('Go to line').click();
        await terminal.press('Home');
        await paste(terminal, '%');
        await terminal.getByText('outdated').expect();
        await prefix(terminal, 'F');
        await terminal.getByText('Rebuild before asking').expect();

        await terminal.resize(160, 45);
        await terminal.press('Ctrl+z');
        await prefix(terminal, 'F');
        await terminal.getByText('Review the message').expect();
        assert.match(await terminal.text(), /Help fix this LaTeX build error/);
        await terminal.screenshot(join(ARTIFACTS, 'problem-ai-draft.png'));
        // Merely choosing Ask AI must not send a request or change the document.
        await terminal.getByText('Unapproved changes').expect({ not: true });
    });
});


test('an error in an imported file marks its import and stays inside the editor pane', async () => {
    await withTexo('imported-error-inline', async ({ terminal, workspace }) => {
        await terminal.resize(160, 45);
        await createResume(terminal, 'Import error');
        const document = await documentPath(workspace);
        await writeFile(join(dirname(document), 'broken-support.tex'), '\\undefinedDependency\n');
        await terminal.press('Ctrl+a');
        await paste(terminal, RESUME.replace('Terminal workflow resume.', '\\input{broken-support}'));
        await prefix(terminal, 'b');
        await terminal.getByText('Imported at line 3').expect();
        await terminal.getByText('Go to line').click();
        await terminal.getByText('3:1').expect();
        await terminal.screenshot(join(ARTIFACTS, 'imported-error-inline.png'));
        assert.match(await terminal.text(), /!3/);
    });
});

test('repeated break/fix cycles retain hidden markers, isolate modal input, and clear old failures', async () => {
    await withTexo('repeated-error-recovery', async ({ terminal }) => {
        await terminal.resize(160, 45);
        await createResume(terminal, 'Recovery');

        for (const command of ['undefinedFirst', 'undefinedSecond', 'undefinedThird']) {
            await terminal.press('Ctrl+a');
            await paste(terminal, RESUME.replace('Terminal workflow resume.', `\\${command}`));
            await prefix(terminal, 'b');
            await terminal.getByText('Undefined control sequence').expect();
            await terminal.getByText('b Retry').expect({ not: true });
            await prefix(terminal, 'D');
            await terminal.getByText('Problems').expect({ not: true });
            await terminal.getByText('!3').expect();

            await prefix(terminal, 'E');
            await terminal.getByText('Error details').expect();
            await terminal.press('End', 'Home', 'PageDown', 'PageUp');
            await paste(terminal, 'This must not edit the document');
            await terminal.resize(75, 28);
            await terminal.press('Escape');
            await terminal.resize(160, 45);
            await terminal.getByText('Error details').expect({ not: true });
            await terminal.getByText('!3').expect();
            await terminal.getByText('This must not edit the document').expect({ not: true });

            await prefix(terminal, 'D');
            await terminal.getByText('Problems').expect();
            await terminal.press('Ctrl+a');
            await paste(terminal, RESUME);
            await prefix(terminal, 'b');
            await terminal.getByText('PDF current').expect();
            await terminal.getByText('Problems').expect({ not: true });
            assert.doesNotMatch(await terminal.text(), /!3/);
        }
    });
});
