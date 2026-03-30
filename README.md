# agent-click

Computer use CLI for AI agents. Control any app from the terminal.

Click buttons, type text, read screens, drag files, move windows, just like how a human would.

Built in Rust. Works on macOS today — Windows and Linux support is underway.

## Installation

### From source

```bash
git clone https://github.com/kortix-ai/agent-click
cd agent-click
./scripts/setup.sh
```

The setup script installs Rust (if needed), builds the CLI, installs it to `~/.cargo/bin/agent-click`, and prompts for accessibility permissions on macOS.

### Cargo

```bash
cargo install --git https://github.com/kortix-ai/agent-click --path cli
```

## Quick start

```bash
agent-click apps                              # what's running?
agent-click snapshot -a Calculator -i -c      # see interactive elements
agent-click click @e5                         # click by ref
agent-click type "hello" -s @e3               # type into a field
agent-click text -a Music                     # read all visible text
```

The workflow: **snapshot → identify refs → act → re-snapshot to verify**.

## Commands

### Discovery

```bash
agent-click apps                              # list all running apps
agent-click snapshot -a Music -i -c           # interactive elements, compact
agent-click snapshot -a Safari -d 8           # deeper tree
agent-click tree -a Finder                    # raw accessibility tree (JSON)
agent-click find 'role=button' -a Calculator  # find matching elements
agent-click find 'id="play"' -a Music        # find by id
agent-click find 'name~="Submit"' -a Safari  # find by partial name
agent-click get-value @e5                     # read element value/state
agent-click text -a Calculator                # all visible text
agent-click focused                           # currently focused element
agent-click windows -a Finder                 # list windows with positions
```

### Click

```bash
agent-click click @e5                         # click by ref (AXPress, no focus steal)
agent-click click 'name="Login"' -a Safari   # click by selector
agent-click click 'id~="track-123"' -a Music # partial id match
agent-click click @e5 --count 2              # double-click
agent-click click @e5 --button right         # right-click
agent-click click --x 500 --y 300 -a Finder  # coordinate click (last resort)
agent-click click @e5 --expect 'name="Done"' # click then verify element appeared
```

### Type

```bash
agent-click type "hello" -s @e3               # type into element (AXSetValue)
agent-click type "hello" -a Safari            # type into focused field (keyboard sim)
agent-click type "hello" -s @e3 --append      # append without clearing
agent-click type "hello" -s @e3 --submit      # type then press Return
```

### Key

```bash
agent-click key Return -a Calculator          # press a key
agent-click key cmd+c -a TextEdit             # key combo
agent-click key cmd+shift+p -a "VS Code"      # complex combo
agent-click key Escape -a Slack               # escape
agent-click key space -a Music                # play/pause
```

### Scroll

```bash
agent-click scroll down -a Music              # scroll the main content area
agent-click scroll down --amount 10 -a Music  # scroll more
agent-click scroll-to @e42                    # scroll element into view
```

### Drag

```bash
agent-click drag @e5 @e10 -a Finder                           # drag by refs
agent-click drag 'name="file.txt"' 'name="Desktop"' -a Finder # drag by name
agent-click drag --from-x 200 --from-y 55 --to-x 900 --to-y 300 -a Finder  # by coordinates
```

Drag uses smooth 20-step interpolation with easing — mimics natural mouse movement.

### Window management

```bash
agent-click move-window -a Notes --x 100 --y 100    # move window (instant)
agent-click resize-window -a Notes --width 800 --height 600  # resize (instant)
agent-click open Calculator --wait                   # launch and wait for ready
agent-click screenshot -a Music --path shot.png      # screenshot an app
agent-click screenshot --path full.png               # full screen
```

### Wait and verify

```bash
agent-click wait-for 'name="Dashboard"'              # poll until element appears
agent-click wait-for 'role=button' --timeout 15      # custom timeout
agent-click ensure-text @e3 "hello"                  # only types if value differs
```

### Batch and workflow

```bash
echo '[["click","@e5"],["key","Return","-a","Music"]]' | agent-click batch
echo '[["click","@e5"]]' | agent-click batch --bail  # stop on first error
agent-click run workflow.yaml                         # execute YAML workflow
```

### System

```bash
agent-click check-permissions                 # verify accessibility access
agent-click observe -a Calculator             # interactive TUI explorer
```

## Selectors

### Refs (fastest — from latest snapshot)

```bash
@e1, @e2, @e3
```

### Selector DSL

```bash
'role=button'                          # by role
'name="Login"'                         # exact name
'name~="Log"'                          # name contains (case-insensitive)
'id="submit-btn"'                      # exact id
'id~="track-123"'                      # id contains
'button "Submit"'                      # shorthand: role name
'"Login"'                              # shorthand: just name
'role=button index=2'                  # 3rd button (0-based)
'css=".my-button"'                     # CSS selector (CDP only)
```

### Chains (parent → child)

```bash
'id=sidebar >> role=button index=0'    # first button inside sidebar
'name="Form" >> button "Submit"'       # submit inside form
```

## CDP (Electron apps)

Electron apps (Slack, Cursor, VS Code, Postman, Discord, Notion) get automatic CDP support. agent-click detects Electron apps, auto-relaunches them with a debug port, and connects via WebSocket.

```bash
agent-click text -a Slack                     # just works — auto-detects Electron
agent-click snapshot -a Slack -i -c           # DOM tree merged with native shell
agent-click click @e5                         # JS element.click() via CDP
agent-click key cmd+k -a Slack                # CDP Input.dispatchKeyEvent
agent-click type "hello" -a Slack             # CDP Input.insertText
agent-click scroll down -a Slack              # JS scrollBy()
```

First run auto-relaunches the Electron app with CDP (~5s). Every subsequent run uses cached connection (~15ms).

Override auto-detection:

```bash
agent-click snapshot -a MyApp --cdp --cdp-port 9222   # force CDP with specific port
agent-click snapshot -a Slack --no-cdp                 # disable CDP, use native only
```

## Output

All output is JSON by default.

```bash
agent-click click @e5                         # {"success": true, "message": "pressed ..."}
agent-click click @e5 --human                 # human-readable output
agent-click click @e5 --compact               # single-line JSON
```

## Architecture

```
agent-click/
├── cli/                    Rust CLI
│   ├── src/                Binary, handlers, actions, snapshot, wait, DSL parser
│   └── crates/
│       ├── agent-click-core/      Platform trait, AccessibilityNode, Selector, element utils
│       ├── agent-click-macos/     macOS backend (AXUIElement, CGEvent, batch attribute fetch)
│       ├── agent-click-cdp/       CDP bridge (WebSocket, DOM walker, JS interaction)
│       ├── agent-click-linux/     Linux backend (AT-SPI2) [planned]
│       └── agent-click-windows/   Windows backend (UIAutomation) [planned]
├── docs/                   Next.js docs site
├── benchmark/              E2E benchmarks, stress tests, comparison tools
├── docker/                 Cross-compilation Dockerfiles
└── scripts/                Setup, build, bench
```

## Development

```bash
pnpm build          # build CLI (release)
pnpm test           # run tests
pnpm lint           # cargo fmt --check + clippy
pnpm format         # auto-format everything
pnpm bench          # criterion micro-benchmarks
pnpm bench:e2e      # real-world app benchmarks
pnpm bench:stress   # reliability stress test
pnpm bench:diff     # compare benchmark runs
```

## Platform support

| Platform               | Status      |
| ---------------------- | ----------- |
| macOS (Apple Silicon)  | Preview     |
| macOS (Intel)          | Preview     |
| Electron apps (CDP)    | Preview     |
| Windows (UIAutomation) | In progress |
| Linux (AT-SPI2)        | In progress |

## License

MIT
