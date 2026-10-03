# Codex Model Catalog Contract Evidence

> Historical schema evidence. The null/empty implementation choice below was replaced on 2026-10-02 after verifying Codex 0.159.3 selector behavior; see [updated evidence](codex-reasoning-selector-defaults-2026-10-02.md) and [ADR-0053](../adr/0053-common-reasoning-selector-defaults.md).

- Investigation date: 2026-09-27
- Local Codex CLI: `codex-cli 0.157.1`
- npm package: `@openai/codex@0.157.1`
- Official source tag: `rust-v0.157.1`
- Official source base: `https://raw.githubusercontent.com/openai/codex/rust-v0.157.1/`

## Conclusion

The Codex version installed on this machine is 0.157.1. Its model catalog entries use `ModelInfo` with these reasoning fields:

- `default_reasoning_level: Option<ReasoningEffort>`
- `supported_reasoning_levels: Vec<ReasoningEffortPreset>`

`supported_reasoning_levels` has no serde default. Omitting it is therefore not an official representation of an unrestricted selector range; deserializing an incomplete entry can fail. The unknown-model fallback explicitly supplies:

- `default_reasoning_level: None`
- `supported_reasoning_levels: Vec::new()`

GPTEasy serializes `null` and an empty array for models without official capability evidence to match the required field contract and the Codex unknown-model fallback. The empty array is not interpreted as all effort values being supported.

## Source evidence

The investigation used the local version output, package metadata, the local Codex source copy, and the corresponding official source paths:

- `codex-rs/protocol/src/openai_models.rs`: `ModelInfo` and reasoning fields.
- `codex-rs/protocol/src/openai_models.rs`: `ReasoningEffort` and `ReasoningEffortPreset` serialization.
- `codex-rs/core/src/models_manager/manager.rs`: conservative fallback construction for unknown models.
- Local investigation record: `C:\Users\yinsh\.codex\backups\terminal-no-popup-20260927\diagnosis.md`.

Official source links:

- `https://raw.githubusercontent.com/openai/codex/rust-v0.157.1/codex-rs/protocol/src/openai_models.rs`
- `https://raw.githubusercontent.com/openai/codex/rust-v0.157.1/codex-rs/core/src/models_manager/manager.rs`

## Implementation choice

`src-tauri/src/provider/model_catalog.rs` keeps complete catalog entries and writes the following for supplier models without official capability evidence:

```json
{
  "default_reasoning_level": null,
  "supported_reasoning_levels": []
}
```

The root `model_reasoning_effort` is not derived from catalog capability fields. Only `api.openai.com` and `api.deepseek.com` currently use an explicit, versioned `high` mapping; other hosts omit effort. The shared mapping is in `src-tauri/src/provider/reasoning.rs` and is used by Windows, WSL2, Bash, and Zsh.

## Verification boundary

This record covers version, source, and schema verification. It does not claim that a real Codex selector UAT has been completed, that an empty array means unrestricted selection, or that real supplier requests proved every model supports `low`, `medium`, `high`, or `xhigh`. Selector behavior and a real DeepSeek Responses request remain separate acceptance work.
