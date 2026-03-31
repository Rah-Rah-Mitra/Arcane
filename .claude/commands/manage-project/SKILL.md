# Skill: manage-project

Inspect, tag, and remove sources within an Arcane project.

## When to use

- User wants to "list projects", "show project", "what's in a project"
- User wants to "tag", "label", or "categorize" a source
- User wants to "remove" or "delete" a source from a project
- User wants to see what chunks exist for a source

## Commands reference

### List all projects
```bash
arcane list
```

### Show project details (sources, chunks, tags)
```bash
arcane show "<project>"
```

### Tag / untag a source
```bash
arcane tag "<project>" "<source-title>" "<tag>"
arcane untag "<project>" "<source-title>" "<tag>"
```

### List chunks for a project
```bash
arcane list-chunks "<project>"
```

### Remove a source
```bash
arcane remove "<project>" "<source-title>"
```
> Confirm with the user before running `remove` — this deletes the source record and its chunks from the database. The original PDF is NOT deleted.

## Steps Claude must follow

1. **Identify the operation** from the user's request (list / show / tag / remove / list-chunks).

2. **Run the appropriate command** from the reference above.

3. **For `remove`**: always confirm with the user first. Show the source title and project name. Only proceed when the user confirms.

4. Fill in `template.md` and present the summary.

## Notes

- Tags are free-form strings. Common conventions: topic, year, edition (`"2024"`, `"ml"`, `"3rd-ed"`).
- `arcane show` is the fastest way for an agent to understand a project's current state before planning further operations.
