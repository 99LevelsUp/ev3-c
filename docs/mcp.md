# EVƎ C's MCP server

`ev3 c mcp` (or `ev3-c mcp`) serves EVƎ C's project operations as MCP tools over standard input
and output, so an agent can build a C project for the EV3, run it and read its results.

It complements EVƎ Cockpit's own MCP server (`ev3 mcp`), which handles bricks, files, stopping a
program and telemetry. Register both with an agent that works on EV3 projects; EVƎ C's tools
repeat none of the cockpit's.

## Registering it

Install the `ev3-c` program first (see [cli.md](cli.md)). Then, for example in Claude Code:

```
claude mcp add ev3-c -- ev3-c mcp
```

## Tools

| Tool | What it does |
|---|---|
| `build_project` | Builds the project; returns the artifacts and any compiler warnings, or the errors. |
| `upload_project` | Builds and uploads what changed, without running. |
| `run_project` | Builds, uploads and runs, waits for the run to end, and returns how it ended, its output and the files it returned (downloaded into `returned/run-<n>/`). Cancelling the call stops the program. |
| `list_project_runs` | The project's runs on the brick and how each ended. |
| `show_project_run` | One run's result, with its returned files downloaded. |
| `delete_project_runs` | Deletes the run folders, all or all but the newest `keep`. |

Every tool takes `project`, the folder that holds `ev3c.json`, and most take `brick`, by name or
id. Without `brick`: the brick EVƎ Cockpit's VS Code window shows, else the only connected brick.
Each result is the object `ev3 c <command> --json` prints, as text and as structured content; a
failure is a tool error carrying `{ "error": { "message", "code"? } }` with the cockpit's code.

## Rules

- **Folders:** a project folder must lie inside the roots the MCP client declares, or inside
  the folder the server was started in when it declares none.
- **Connecting:** nothing reaches EVƎ Cockpit's daemon until a tool needs a brick. Then the server
  connects as kind `mcp`, named after the agent ("Claude Code (EVƎ C MCP)"), so other clients
  see who holds the brick. An agent can never confirm anything on a person's behalf.
- **Output:** standard output carries MCP messages only, one JSON-RPC message per line.
