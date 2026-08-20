# AGENTS

<!-- lane:protocol -->
## Context memory

- Before editing a file, read `.context/-/<path>/` if it exists, or run `lane why <path>`.
- Record non-obvious findings with `lane note -p <path> -a <anchor> "..."`.
- Do not edit `.context/` by hand; `lane done` manages it.
- Detailed workflow lives in the `lane` skill; run `lane install skill` if it is absent.
<!-- /lane:protocol -->
