# Playtest 3: a person and an AI assistant on the same sound

Phase 2, step 10. The question: can a person in Duck Echo Studio and an
AI assistant through the MCP server work on one sound at the same time,
see each other's changes, and undo them, without losing anything?

## How it was played

* **The assistant** was the real `duck-echo-mcp` program, started as a
  child process and spoken to over stdin and stdout, exactly as Claude
  Desktop starts it. Claude played the assistant, asking for the tools an
  assistant would use (`list_files`, `sound_set`, `song_add_note`,
  `edit_history`).
* **The person** was the real Studio code, drawn with no window
  (`egui_kittest`): the same screens, clicks and keys, but no speakers
  and no real window in the cloud machine the playtest ran on.
* The two shared nothing but the project folder, as on a real computer.
* The session is written down as a test,
  `studio/crates/app/tests/together.rs`, so CI plays it again on Windows
  and Linux at every push.

## What happened

| # | Who | Did | What the other saw |
|---|---|---|---|
| 1 | Person | Made a copy of `coin` and opened it | |
| 2 | Assistant | `list_files`, then `sound_set` decay to 0.5 | Within half a second the Studio's text showed `decay = 0.5`, the decay slider moved, and the note "AI assistant: Set layers[0].envelope.decay to 0.5 in coin.toml" showed for three seconds with Undo. The preset's comments were still in the file. |
| 3 | Person | Dragged the volume slider to 0.6 | Both changes were in the file. |
| 4 | Assistant | `edit_history` | The newest step was "You", the one before "AI assistant". |
| 5 | Person | Opened History | Both steps, the assistant's marked in cyan. |
| 6 | Assistant | `sound_set` pitch start to A5 | The note again. |
| 7 | Person | Pressed **Undo** in the note | The pitch went back to B5; the person's volume stayed. |
| 8 | A text editor | Changed the volume to 0.7 | The Studio noted "changed by another program", and the assistant's `edit_history` showed the step "Changed outside Duck Echo". |
| 9 | Person | Ctrl+Z | The outside change went back too. |
| 10 | Assistant | `song_add_note` A5 to the open day theme | The timeline showed the note at bar 2, beat 4, with the note and the History entry. |

## Found and fixed during the playtest

1. **Typing could undo the assistant's change in the file.** Typing is
   saved as the whole text. An assistant's change made while the person
   was typing was saved over: still in History, but gone from the file.
   Now the editor sees the file changed and asks: **Keep mine** (save
   over theirs; theirs stays in History) or **Use theirs**. A test
   covers both answers.
2. **After an Undo, a new change was not noticed.** The history gives
   the next step the number of the step that was undone, so the Studio
   thought it had seen it already, and no note showed. Steps are now
   known by number, time and title together.
3. **Settings had grown tall.** It scrolls now, with Connect an AI
   assistant last, so the older settings stay where they were.

## Still weak

* **The Studio looks twice a second.** An assistant's change shows
  within half a second, not at once. Fine for a person; a file watcher
  could make it instant later.
* **Notes only for the open file.** Home picks up a new or changed file
  within a second, but does not say who changed it.
* **One Undo for everyone.** Ctrl+Z takes back the newest step, whoever
  made it. History and the note say who made it, but a person can still
  undo the assistant's step when they meant their own.
* **The assistant does not hear about the person.** It sees the
  person's steps only when it asks for `edit_history`.
* **Not yet tried with the real Claude Desktop and real speakers.**
  That is the Windows checklist of this step; the Linux one tries the
  same sharing with a text editor.
