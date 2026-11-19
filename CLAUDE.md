@AGENTS.md

Everything for agents is in AGENTS.md, above, so there is one text to
keep right. What follows is only for Claude Code.

## The project's MCP server

`.mcp.json` starts the `duck-echo` server for anyone who opens Claude
Code here. It runs `cargo run --release -q -p duck-echo-mcp` with
`--root` set to `playtest/`, so every file the tools write lands there.

- Build it once before the first session:
  `cargo build --release -p duck-echo-mcp`. A cold build takes longer
  than Claude Code's 30-second start timeout. Or let the first start
  build it, with a longer timeout: `MCP_TIMEOUT=120000 claude`.
- Start Claude Code from the repository's top folder (the server stops
  if it cannot find `playtest/`) and approve the server once. `/mcp` in a
  session, or `claude mcp list`, shows it.
- Playtest results are committed: give each playtest its own folder
  under `playtest/`, with its recipes, its WAVs and a review, as in
  `playtest/playtest-2`. WAV files are committed there, unlike in the
  rest of the repository.
