# AGENTS

<!-- lane:protocol -->
## Context memory

- Before editing a file, read `.lane/memory/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note add <path> -a <anchor> "..."`.
- Do not edit `.lane/` by hand; `lane merge` manages it.
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
<!-- /lane:protocol -->
