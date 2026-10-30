# PolishCast skill

A skill for AI agents that use **PolishCast**, the desktop app that turns screen recordings into polished videos: automatic zoom on clicks, a smooth cursor, a background, padding and rounded corners. The
skill tells the agent how to use PolishCast's tools well: which to call first, what
needs your yes, sensible defaults, and how to report back.

## What you need

- **PolishCast**, installed from <https://mayorana.ch/en/apps/polishcast> (macOS 15+ or
  Windows 10 2004+). This skill does not include the app.
- A desktop agent connected to PolishCast's MCP server: Claude Desktop, Claude
  Code, Codex CLI or Cursor. `references/connect.md` has the steps. Web-only
  agents such as ChatGPT can't reach an app on your computer.

## What you can ask for

- Record a demo of an app while the agent drives it
- Tune a take's zooms, trim it and cut its pauses
- Change the look: background, aspect (16:9, 9:16, 1:1), cursor, speed
- Take background noise out of the voice
- Export MP4 or GIF

## What waits for your yes

- Starting a screen recording (the agent says what it will record first)
- Replacing an existing export with the same name

## Limits

The free version exports 10 different takes. You can turn recording from an agent off in PolishCast's MCP window.

## Install

- **Claude Code**: unzip into `~/.claude/skills/` so the folder is
  `~/.claude/skills/polishcast/`.
- **claude.ai or Claude Desktop**: upload the zip under Skills in Settings.
- **Other agents**: put the `polishcast` folder where the agent reads skills.

## Help

Questions, licences and bug reports: <https://mayorana.ch/en/contact>.
