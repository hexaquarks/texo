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
| Normal and Visual | `h/j/k/l`, arrows, `w/b/e`, `0/^/$`, Home/End, `gg/G`, Page Up/Down |
| Normal | `i/a/I/A/o/O` enter Insert; `v` enters Visual; `V` initially selects the current source line |
| Insert | Ordinary editor input; Escape, Ctrl+[ or Ctrl+C returns to Normal |
| Normal | `d/c/y` followed by a supported motion, text object, `gg`, or `G`; `dd/cc/yy` operate on a whole source line |
| Normal | `x` deletes the character under the cursor |
| Visual | `d/c/y` deletes, changes, or yanks the inclusive character range in either direction; `v` or Escape cancels |
| Normal and Visual | `D/C` delete/change to line end; `S` changes the current line; `D/C/S` act on the selected range in Visual; `J` joins lines; `p` pastes the internal yank buffer at the cursor; `u` undoes; Ctrl+R redoes |
| Normal and Visual | Ctrl+E/Y scroll one row, Ctrl+D/U half a page, Ctrl+F/B a page |

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

`c/cc/C/S` enter Insert mode; deletes and yanks return to Normal. Yanks and cuts also attempt a
system clipboard copy. The standard clipboard alternatives above work in Vim mode.
Pending commands clear on Escape, focus changes, prefix commands, résumé switches, and AI edits.
Unrecognized commands are consumed without inserting them into the source.

This is a small adapter of the upstream example, not full Vim compatibility. Motions use the
component's behavior: `j/k` follow wrapped display rows, `^` means column zero, `gg/G` retain the
column where possible, and `h/l` can cross line boundaries. Yanking leaves the cursor at the
selection endpoint. Whole-line operators include the following newline when it exists; `cc`
removes that line before entering Insert. `V` does not implement Vim's full Visual Line mode.
Undo follows component transactions; replacing a selection, including accepting AI edits, may require
two undos. Counts, registers, sentence/paragraph/tag objects, repeated Visual-object expansion, dot repeat,
replace modes, and Ex commands are unsupported. Delimiter objects use matching characters rather
than a TeX parser.

Mouse selection does not scroll beyond the viewport while dragging. Terminal support determines
which modified shortcuts are distinguishable; the configured prefix always takes precedence.
The mouse and Vim adapters retain the upstream [MIT attribution](../THIRD_PARTY_LICENSES/ratatui-textarea.txt).
