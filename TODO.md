# TODO

## Next Codex Session

- Use `./dev demo-smoke` when touching scene status indicators; it launches Demo mode with the expected visual checkpoints.
- Live WebSocket comparison completed on 2026-04-23: visible chat bubble and silent `NO_REPLY` sparkle paths matched Demo mode.
- Next recommended beat: if scene status indicators change again, rerun `./dev demo-smoke` and a live WS silent probe before merging.
- If indicator behavior regresses, inspect whether the run reached `AgentStatus::Running` and whether a visible bubble is intentionally suppressing the sparkle.
