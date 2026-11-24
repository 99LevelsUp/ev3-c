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
| `start_project_run` | Builds, uploads and starts the program, and returns as soon as it runs, with a run handle (`run`), without waiting for its end. A failed build or start returns the error. |
| `read_project_run` | A started run's stdout and stderr from the given offsets on, whether it still runs, and at the end its result. |
| `stop_project_run` | Stops a started run, as Ctrl+C does; its result then comes with `read_project_run`. |

Every project tool takes `project`, the folder that holds `ev3c.json`, and most take `brick`, by
name or id. Without `brick`: the brick EVƎ Cockpit's VS Code window shows, else the only connected
brick. Each result is the object `ev3 c <command> --json` prints, as text and as structured
content; a failure is a tool error carrying `{ "error": { "message", "code"? } }` with the
cockpit's code.

## Watching a program while it runs

`run_project` answers only when the program has ended. To see what a program prints while it
runs, start it with `start_project_run` and read it with `read_project_run`:

- `read_project_run` takes the `run` handle and `out_offset` and `err_offset`, the `outOffset` and
  `errOffset` the last read returned (0 or none the first time), so each read returns only what
  is new: `out` and `err`.
- While the program runs and nothing is new, the read waits up to `wait_seconds` (default 10, at
  most 30) and then answers with what there is.
- `running` says whether the program still runs. Once it is `false`, `ended` holds the result,
  the object `run_project` returns, without the output, which the reads already gave.
- One read returns at most 32 KiB of each stream; `more: true` says the next read has the rest.
- `messages` holds what EVƎ C said along the way, such as the output coming only at the end when
  the brick has no file follow free (EVƎ Cockpit allows two per brick).
- The server keeps the runs still going and the last eight that ended.

## Rules

- **Folders:** a project folder must lie inside the roots the MCP client declares, or inside
  the folder the server was started in when it declares none.
- **Connecting:** nothing reaches EVƎ Cockpit's daemon until a tool needs a brick. Then the server
  connects as kind `mcp`, named after the agent ("Claude Code (EVƎ C MCP)"), so other clients
  see who holds the brick. An agent can never confirm anything on a person's behalf.
- **Output:** standard output carries MCP messages only, one JSON-RPC message per line.
- **Contract:** the `initialize` result says which EVƎ Cockpit daemon contract the server speaks,
  as `capabilities.experimental["ev3-cockpit"].contract`. It is the contract of the cockpit
  client `ev3-c` is built with, so a host such as the cockpit's own `ev3 mcp` can pass on these
  tools only when it speaks the same.
