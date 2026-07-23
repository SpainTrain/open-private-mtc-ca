# Fixture: intentionally broken Mermaid

This file exists so `make diagrams-check-selftest` can prove the checker actually fails on
invalid Mermaid syntax (E2E smoke, spec §19.13 spirit). Do not fix this diagram.

```mermaid
flowchart TD
    A[Start] --> --> B{{{unclosed
    this is not valid mermaid
```
