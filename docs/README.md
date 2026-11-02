# Documentation

| Document | For | What's in it |
|---|---|---|
| [overview.md](overview.md) | anyone evaluating or inheriting the project | the problem, what netwatch does, the architecture, how it is checked, status |
| [user-manual.md](user-manual.md) | people using it | the screen, statuses, incidents, reports for an ISP, settings, privacy, troubleshooting |
| [../deploy/README.md](../deploy/README.md) | people installing it | the RPM, the install script for other distros, the container |
| [adr/](adr/README.md) | contributors | every design decision, with its context and consequences |
| [glossary.md](glossary.md) | contributors | the domain vocabulary that code, specs, and UI share |
| [context-map.md](context-map.md) | contributors | the bounded contexts and the crates that hold them |

## Diagrams

| Diagram | Shows |
|---|---|
| [monitored-path.svg](diagrams/monitored-path.svg) | the roles netwatch watches along one path |
| [system-context.svg](diagrams/system-context.svg) | netwatch and everything it talks to |
| [architecture.svg](diagrams/architecture.svg) | the crates and their dependencies |
| [data-flow.svg](diagrams/data-flow.svg) | how a measurement travels from the kernel to the browser |
| [erd.svg](diagrams/erd.svg) | the SQLite schema |
| [incident-lifecycle.svg](diagrams/incident-lifecycle.svg) | how a grade becomes an incident, an episode, and a report |

Each SVG is rendered from the Graphviz source beside it (`*.dot`). After editing a
source, run `npm run diagrams` in `web/` and commit both files. A change to the
schema, the crates, or the data flow updates its diagram in the same commit.

Screenshots in [images/](images/) are from a real installation. The public address
and hostname are replaced with documentation values (`203.0.113.7`, `myhost`).
