# The command and agents

The `dermixen` command is a front door to the app. Every engine capability ships as a command, and the command is how each capability is heard and checked from a terminal.

## Intelligence lives outside the app

The workflow that motivates the command is a request like this one, addressed to an agent rather than to the app:

> Whip up a 90-minute mix of tracks in ~/audio/goa/comp from 1996, opening with Slinky Wizard, Lunar Juice (Hallucinogen Moon Strudel Remix).

Dermixen doesn't ship a creative auto-DJ. It ships deterministic operations (query [the library](../library.md), assemble a mix document, adjust it, render it), and you or your agent supply everything else. The agent supplies the selection, the order, and the taste, which is to say your taste as you've expressed it. An agent satisfying that request scans and queries the library, chooses an order using the tempo and Camelot data in the JSON results, builds the mix with the `mix` commands or by writing the project file directly, and renders it or hands it to the window with `open`. Automation happens through the app and never inside it, which is why the app stays deterministic and why `DESIGN.md` lists an auto mode among the non-goals.

## What the command promises a program

- Every command prints one JSON document behind `--json`, and `docs/json/dermixen.schema.json` fixes the shape of each. Human-readable text is the default, and the JSON is one flag away.
- Exit codes mean something: 0 for success, 1 for a failure with one `error:` line on standard error, 2 for a command line that couldn't be parsed.
- Progress and warnings go to standard error, so standard output contains only the result.
- The project file is a public contract: versioned JSON, described field by field, that a person or a program may write directly. The command validates it and names the field that's wrong.

## A tracklisting is a program's input too

In a second workflow, you hand the agent a complete tracklisting and ask for that exact set. `dermixen library find` is the operation that makes it work: fuzzy matching over tags and parsed file names, returning ranked candidates with scores. The agent resolves each line to a file, reports the lines it couldn't resolve instead of guessing, and builds the mix in the given order. [Build a mix from a tracklisting](../how-to/build-a-mix-from-a-tracklisting.md) shows the script.

## The command is the verification harness

Because every capability has a command, every part of the engine can be heard: render a crossfade, play it, compare a captured preview against a render. You can verify the app by running commands and listening, whether or not you also read the code, and the command is what makes that possible. [Drive Dermixen from a script](../how-to/drive-dermixen-from-a-script.md) covers the practical side, and [The `dermixen` command](../cli.md) is the reference.
