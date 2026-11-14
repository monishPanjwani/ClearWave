# Repository Guidelines

## Project Structure & Module Organization

This workspace currently contains no application source, tests, assets, or build configuration. No language or framework has been established. The hidden `.agents/`, `.codex/`, `.aws/`, and `.git/` directories are environment metadata; do not use them for application code or modify them as part of ordinary contributions.

When adding the initial implementation, choose a structure appropriate to the selected stack. Prefer clear boundaries such as `src/` for source, `tests/` for tests, and `assets/` for static resources where applicable. Document the resulting layout in `README.md`.

## Build, Test, and Development Commands

No build, test, lint, or development commands are configured yet. Do not assume commands such as `npm test` or `make build` are available.

When introducing tooling, provide reproducible commands for installing dependencies, running locally, building, testing, and checking formatting. Record them in `README.md` and commit the relevant configuration and dependency lockfile.

## Coding Style & Naming Conventions

Follow the conventions of the language and framework introduced by the first implementation. Use consistent indentation, descriptive names, and small modules with focused responsibilities. Configure a formatter and linter appropriate to the chosen stack, and keep formatting changes separate from behavior changes when practical.

## Testing Guidelines

No test framework or coverage threshold exists yet. Add tests alongside new behavior using the selected framework's discovery conventions. Cover expected behavior, failure cases, and regressions. Document the exact test command and any required fixtures; keep tests independent of personal credentials.

## Commit & Pull Request Guidelines

No readable Git history is available to establish existing commit conventions. Use concise, imperative commit subjects, such as `Add initial project structure`, and keep commits focused.

Pull requests should explain the change, link related issues when available, and describe validation performed. Include screenshots for visible interface changes and identify any setup or migration steps.

## Security & Configuration

Keep secrets, credentials, and local environment files out of version control. Provide configuration examples with placeholder values and document required environment variables.
