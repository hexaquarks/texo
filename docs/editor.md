# TeX editor commands

Standard editing is the default. Open Settings with the configured prefix followed by `s`
(default Ctrl+X, then S), select **Standard** or **Vim**, and press Enter to apply. Ctrl+V toggles
the choice within Settings; Escape closes without applying it.

You can also use **Open config**, then **Reload** after changing:

```toml
[editor]
mode = "standard" # or "vim"
soft_wrap = true
```

The preference affects the TeX editor; chat keeps its ordinary input behavior. Each résumé retains
its buffer, cursor, selection, search, scroll position, and undo history while Texo is running.

Application commands use the configured prefix (default Ctrl+X), followed by a key:
`w` save, `b` build, `p` Actions, `e` export, `s` Settings, `c` cycle panes, or `h/j/k/l` select a pane.
During AI review, prefix `a/m/d` accepts, modifies, or discards the proposal.

## Standard editing

Shift+navigation selects text. Ctrl+Left/Right and Alt+Left/Right move by words;
Home/End move to line boundaries, and Page Up/Down scroll. Ctrl+A selects all;
Ctrl+Shift+A is available when the prefix owns Ctrl+A. Ctrl+Z undoes;
Ctrl+Y or Ctrl+Shift+Z redoes. Ctrl+Shift+U/R provide undo/redo alternatives if the prefix owns a binding.
Ctrl+S saves; prefix `w` also works when the prefix owns Ctrl+S.

Ctrl+C/X/V copies, cuts, or pastes through the system clipboard. Ctrl+Insert, Shift+Delete,
and Shift+Insert provide alternatives. Clipboard failures appear in the status row;
Alt+Insert always pastes the component's internal yank buffer. Bracketed paste inserts literal
multiline text in either editing mode. With wrapping off, Ctrl+Alt+Left/Right pans horizontally.
Click to position the cursor; drag to select within the editor viewport.

Ctrl+F (Ctrl+Shift+F in either mode) opens literal search. Enter and Shift+Enter find the next
and previous match, wrapping through the document. Escape closes search and clears its highlights.
Search works within individual source lines; newlines pasted into the query become spaces.

## Supported Vim subset

| Mode | Commands |
| --- | --- |
| Normal and Visual | `h/j/k/l`, arrows, `w/b/e`, `W/B/E`, `0/^/$`, Home/End, `gg/G`, Page Up/Down |
| Normal | `i/a/I/A/o/O` enter Insert; `v` enters Visual; `V` enters Visual Line; `j/k` extend/shrink the selection by whole source lines |
| Insert | Ordinary editor input; Escape, Ctrl+[ or Ctrl+C returns to Normal |
| Normal | `d/c/y` followed by a supported motion, text object, `gg`, or `G`; `dd/cc/yy` operate on a whole source line |
| Normal | `x` deletes characters; `r` replaces characters; `s` substitutes characters and enters Insert |
| Visual | `d/c/y` deletes, changes, or yanks the inclusive character range in either direction; `v` or Escape cancels |
| Normal and Visual | `D/C` delete/change to line end; `S` changes the current line; `D/C/S` act on the selected range in Visual; `J` joins lines; `p/P` paste after/before the cursor (or below/above for linewise yanks); `u` undoes; Ctrl+R redoes |
| Normal and Visual | Ctrl+E/Y scroll one row, Ctrl+D/U half a page, Ctrl+F/B a page |

Counts work with motions, operators, find commands, line operations, character edits, paste, and
Normal-mode undo: for example `3j`, `2dd`, `d2j`, `2d3w`, `3G`, and `2rX`. Counts are bounded by
the component's 16-bit cursor coordinates. `f/F/t/T` find a character on the source line;
`;` repeats that find and `,` reverses it. `Y` yanks whole lines.

`/` and `?` open forward/backward **literal** search. Enter confirms, Escape restores the previous
cursor and search, and `n/N` repeat in the same/opposite direction. Search uses the component's
existing search engine and records jump history.

`.` repeats the last completed Normal-mode edit, including its Insert session or bracketed paste.
A count on `.` replaces the original command count. Focus changes cancel incomplete recordings;
replacing the document clears repeat state. Visual edits are not recorded for dot repeat.

### Text objects

Use `i` for the inside of an object and `a` to include its surrounding space or delimiters.
These work after `d`, `c`, and `y`, and in Visual mode after `v`. For example, `ciw` replaces
the current word and enters Insert, `diw` deletes it, `yiw` yanks it, and `viw` selects it.

| Object | Keys |
| --- | --- |
| Word or punctuation run | `iw`, `aw` |
| WORD separated only by whitespace | `iW`, `aW` |
| Parentheses | `i(`/`i)`/`ib`, `a(`/`a)`/`ab` |
| Braces | `i{`/`i}`/`iB`, `a{`/`a}`/`aB` |
| Square brackets | `i[`/`i]`, `a[`/`a]` |
| Angle brackets | `i<`/`i>`, `a<`/`a>` |
| Double quotes, single quotes, backticks | `i"`/`a"`, `i'`/`a'`, `` i` `` / `` a` `` |

Word objects handle Unicode letters and combining marks. Around-word objects prefer trailing
space, otherwise leading space. Brackets and braces nest and can span source lines; escaped
delimiters are skipped. Quote objects stay on one line and skip escaped quotes.

### Jump history

In Normal mode, **Ctrl+O** visits an older jump and **Ctrl+I** visits a newer one. Traditional
terminals send Ctrl+I as **Tab**, so Tab also moves forward in Normal mode. Tab keeps inserting
text in Standard and Insert modes. The configured application prefix takes precedence.

`gg`, `G`, and search navigation record jumps. Typing a search query records one starting location
when you navigate or close the search, rather than one entry per keystroke. Ordinary arrow and
`h/j/k/l` movement does not add entries. Each résumé keeps up to 100 locations during the session.
Switching résumés preserves its history; accepting an AI replacement or reloading source clears it.
Locations are saved coordinates, clamped after edits if needed, rather than marks that follow
inserted or deleted text. History does not navigate between résumés.

`c/cc/C/S` enter Insert mode; deletes and yanks return to Normal. Yanks and cuts also attempt a
system clipboard copy. The standard clipboard alternatives above work in Vim mode.
Pending commands clear on Escape, focus changes, prefix commands, résumé switches, and AI edits.
Unrecognized commands are consumed without inserting them into the source.

This is a bounded Vim adapter, not full Vim compatibility. `j/k` move by source lines across
soft wrapping; `h/l` stay within a line, `^/I` use the first nonblank character, and `gg/G` target
source lines rather than the textarea's trailing-newline row. `V` retains a whole-line anchor
when extending, shrinking, or reversing the selection. `cw` leaves following whitespace intact;
`cc` leaves a line to replace. Source text, selection, rendering, and history remain owned by
ratatui-textarea.

Undo follows component transactions: an Insert session, selection replacement, or AI replacement
may require multiple undos. Counts on Insert commands/text objects, Visual dot repeat, named
registers, macros, marks, Visual Block, replace mode, paragraph/sentence/tag objects, indentation
operators, and Ex commands are outside this subset. Delimiter objects use matching characters
rather than a TeX parser.

Mouse selection does not scroll beyond the viewport while dragging. Terminal support determines
which modified shortcuts are distinguishable; the configured prefix always takes precedence.
The mouse and Vim adapters retain the upstream [MIT attribution](../THIRD_PARTY_LICENSES/ratatui-textarea.txt).
