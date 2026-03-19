---
name: tickrs
description: CLI for TickTick task management — create, list, update, complete, and delete tasks and projects. Use when asked to manage to-do items, tasks, reminders, or project lists in TickTick.
license: MIT
---

# tickrs — TickTick CLI Skill

Use the `tickrs` command-line tool to manage tasks and projects in TickTick.

## Prerequisites

**Install tickrs:**

```bash
# macOS (Apple Silicon)
curl -LO https://github.com/SpaceK33z/tickrs/releases/latest/download/tickrs-aarch64-apple-darwin.tar.gz
tar -xzf tickrs-aarch64-apple-darwin.tar.gz && sudo mv tickrs /usr/local/bin/

# macOS (Intel)
curl -LO https://github.com/SpaceK33z/tickrs/releases/latest/download/tickrs-x86_64-apple-darwin.tar.gz
tar -xzf tickrs-x86_64-apple-darwin.tar.gz && sudo mv tickrs /usr/local/bin/

# Linux
curl -LO https://github.com/SpaceK33z/tickrs/releases/latest/download/tickrs-x86_64-unknown-linux-gnu.tar.gz
tar -xzf tickrs-x86_64-unknown-linux-gnu.tar.gz && sudo mv tickrs /usr/local/bin/
```

**Authenticate:** Set environment variables and run `tickrs init` (opens browser for OAuth):

```bash
export TICKTICK_CLIENT_ID="your_client_id"
export TICKTICK_CLIENT_SECRET="your_client_secret"
tickrs init
```

For CI/automation, set `TICKTICK_TOKEN` directly instead.

## Key Conventions

- **Always use `--json`** when you need to parse output — the human-readable format is not machine-parseable.
- **Use `--force`** on delete commands to avoid interactive confirmation prompts.
- **Use `--quiet`** when you only need to check success/failure via exit code.
- **Set a default project** with `tickrs project use <name>` to avoid repeating `--project-id`.
- **Use `$'...'` syntax for multi-line content** — bash will interpret `\n` as actual line breaks (see examples below).

## Multi-line Content

When passing multi-line text to `--content`, use bash's `$'...'` syntax to properly interpret `\n` as line breaks:

```bash
# Use $'...' for multi-line content
tickrs task create --title "Follow up" --content $'Key points:\n- First point\n- Second point' --project-id abc123

# Or concatenate with $'\n'
tickrs task create --title "Notes" --content "Line 1"$'\n'"Line 2" --project-id abc123
```

## Common Workflows

### List projects and pick one

```bash
tickrs project list --json
tickrs project use "Work"
```

### Create a task

```bash
# Basic
tickrs task create --title "Review PR" --json

# With priority and due date
tickrs task create --title "Submit report" --priority high --date "tomorrow"

# With tags and subtasks
tickrs task create --title "Pack for trip" --tags "travel" --items "Passport,Clothes,Chargers"

# In a specific project
tickrs task create --title "Research" --project-id abc123 --content "Look into frameworks"
```

### List and filter tasks

```bash
tickrs task list --json
tickrs task list --priority high --status incomplete
tickrs task list --project-id abc123 --tag "urgent"
```

### Complete or update a task

```bash
tickrs task complete <task-id>
tickrs task update <task-id> --title "Updated title" --priority medium
tickrs task uncomplete <task-id>
```

### Delete a task or project

```bash
tickrs task delete <task-id> --force
tickrs project delete <project-id> --force
```

### Manage projects

```bash
tickrs project create --name "Side Project" --color "#00AAFF"
tickrs project show <project-id>
tickrs project update <project-id> --name "Renamed" --closed
```

## Natural Language Dates

The `--date` flag accepts:

| Expression | Meaning |
|------------|---------|
| `today` | Today at current time |
| `tomorrow` | Tomorrow at current time |
| `next week` | 7 days from now |
| `in 3 days` | 3 days from now |
| `in 2 hours` | 2 hours from now |

ISO 8601 also works: `2026-01-15T14:00:00Z`

## JSON Output Format

Success responses look like:

```json
{
  "success": true,
  "data": {
    "tasks": [{ "id": "abc", "title": "...", "priority": 3, ... }],
    "count": 1
  }
}
```

Error responses:

```json
{
  "success": false,
  "error": {
    "code": "AUTH_REQUIRED",
    "message": "Authentication required. Run 'tickrs init' to authenticate."
  }
}
```

### Common error codes

| Code | Action |
|------|--------|
| `AUTH_REQUIRED` | Run `tickrs init` |
| `AUTH_EXPIRED` | Run `tickrs init` again |
| `NOT_FOUND` | Check the task/project ID |
| `NO_PROJECT` | Set a default or pass `--project-id` |
| `RATE_LIMITED` | Wait and retry |

## Automation Script Example

```bash
#!/bin/bash
result=$(tickrs task create --title "Automated task" --json)
task_id=$(echo "$result" | jq -r '.data.task.id')
if [ "$task_id" != "null" ]; then
    echo "Created task: $task_id"
else
    echo "Failed to create task"
    exit 1
fi
```
