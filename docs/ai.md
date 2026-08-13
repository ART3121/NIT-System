# NIT AI

`nit-ai` is the optional local inference adapter used for Roadmap generation.
It is not a general chat system, agent framework, source of truth, or persistence
layer.

## Boundary

The adapter accepts a Core `Entry` and returns either a validated Core `Roadmap`
or a result indicating that the selected model must be downloaded. It does not
receive a `Workspace`, repository, or storage path and therefore cannot write
NIT data directly.

```text
Core Entry → bounded prompt → local Ollama → schema validation → Core Roadmap
```

The CLI or TUI owns user interaction around this operation. It displays
progress, requests permission before a model pull, presents the proposal, and
persists only after explicit acceptance.

Persistence follows the selected `NitApi`: a Plain workspace stores the
accepted Roadmap in readable Markdown, while a Vault workspace commits it as
authenticated ciphertext through Session. AI itself cannot distinguish or
bypass those backends.

## Runtime behavior

- Ollama is contacted only through an endpoint that resolves to an IPv4 or IPv6
  loopback address. A remote `OLLAMA_HOST` is rejected instead of receiving an
  entry over unencrypted HTTP.
- The default model is `qwen3:1.7b`; `NIT_AI_MODEL` can select another local
  compatible model.
- Thinking is disabled for the normal Roadmap operation.
- Context and output limits are bounded for short NIT entries.
- Structured JSON is requested and validated before conversion to a Roadmap.
- Superficial or malformed output may receive one corrective attempt.
- Requests have finite server and generation timeouts.
- Subprocess probes and model downloads have finite timeouts and are terminated
  and reaped after cancellation or timeout.
- HTTP headers, encoded responses, decoded chunked bodies, and error messages
  have explicit memory bounds; chunk arithmetic is checked.
- The TUI uses a cancellable worker so terminal rendering remains responsive.
- The TUI joins a completed or cancelled worker instead of leaving detached
  background work behind.
- `keep_alive` asks Ollama to retain the model briefly for nearby operations;
  Ollama remains responsible for model process and memory lifecycle.

## Failure contract

If Ollama is unavailable, the model is absent, generation times out, output is
invalid, or the user rejects the proposal, durable storage remains unchanged.
AI is an enrichment path after an entry already exists, never a prerequisite
for capture or reading.

## Unix-oriented role

NIT AI behaves like an adapter around an external specialist. It interprets a
small input and returns a typed proposal, while Core remains authoritative. The
project does not embed model management into storage, keep a NIT AI daemon, or
send the whole workspace as ambient context.

See [Architecture](architecture.md#ai-and-external-editing) for the complete
acceptance flow.
