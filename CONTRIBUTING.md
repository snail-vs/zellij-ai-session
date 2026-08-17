# Contributing

Thanks for helping improve zellij-ai-session. Bug reports, compatibility results, documentation improvements and new agent adapters are all welcome.

## Before opening an issue

- Search existing issues first.
- Use the provided issue template.
- Include the zellij-ai-session release, Zellij version, operating system, architecture and affected agent.
- Do not upload raw session files, prompts, database files or credentials.
- Redact usernames, home-directory paths, private repository names and secrets from examples.

## Development setup

Requirements:

- a current stable Rust toolchain;
- the `wasm32-wasip1` target;
- Zellij for interactive plugin testing.

```bash
rustup target add wasm32-wasip1
cargo build -p zellij-ai-session-index --release
cargo build -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm \
  --release
```

Install the local build without downloading release artifacts:

```bash
./install.sh --from-source
```

## Adding an agent adapter

An adapter is appropriate only when the agent's stored sessions can be mapped reliably to a project directory and resumed through a documented CLI command.

1. Add an `AgentMeta` entry in `crates/core/src/lib.rs`.
2. Implement `AgentAdapter` under `crates/indexer/src/`.
3. Register the adapter in the indexer.
4. Add fixtures that contain no real prompts, paths or credentials.
5. Test discovery, title fallback, project mapping and resume command generation.
6. Update both `README.md` and `README.zh-CN.md`.

Keep agent-specific storage parsing inside its adapter. The shared UI should remain agent-independent and project-first.

## Validation

Run the same checks used by CI:

```bash
cargo fmt --all --check
cargo test --workspace
cargo check -p zellij-ai-session-plugin \
  --target wasm32-wasip1 \
  --features wasm
```

Documentation-only changes should still keep Markdown links, commands and English/Chinese feature descriptions consistent.

## Pull requests

- Keep each pull request focused on one problem.
- Explain the user-visible behavior and why the change is needed.
- Add or update tests for behavior changes.
- Mention affected agents and platforms.
- Call out changes that read additional local data, modify Zellij configuration or launch new commands.

By contributing, you agree that your contribution is licensed under the repository's MIT License.
