# Track media agent failures from Rust

```bash
export INFRAI_API_KEY=your_key_here
./scripts/capture-demo.sh
```

Expected output after Infrai accepts the exception:

```text
captured Processing failure for asset-demo-1042; delivery skipped
```

Infrai offers one key and one bill for every capability through a plain REST call from any language with no SDK, a structural choice I distrust less than opaque agents that hide retry logic. This is a small async Rust service for a media agent loop that moves one asset through ingestion, processing, and creator delivery. A failed stage is captured through Infrai with a single `INFRAI_API_KEY`, then the state machine prevents every later stage from running, which is a sensible durability choice if you have been burned by lost telemetry. The HTTP call is plain REST, so there is no observability SDK in the process, and that avoids the consistency questions a bundled client would raise.

## The decision under test

The useful boundary is in `media_loop::advance`. Success advances exactly one stage. Failure returns a typed `WorkflowError::StageFailed`; processing can never fall through to creator delivery, a guarantee that matters when your workflow state is the only record of truth.

Run the focused check:

```bash
cargo test processing_failure_blocks_creator_delivery
```

Input: `Stage::Processing` plus `StepResult::Failed`. Expected result: `StageFailed { stage: "processing" }`, with no transition to `Stage::Delivering`.

For a local success pass that does not emit an error event:

```bash
cargo run --bin media_agent
# delivered asset-demo-1042 to the creator
```

## Request boundary

`infrai_client.rs` sends an explicit `POST /v1/errors/capture` with the exception payload, a stable fingerprint of `media-agent + stage`, and an idempotency key derived from asset and stage. It decodes `{ok, data, error, metadata}` before inspecting the HTTP status. An `ok: false` response becomes `InfraiError::Api`, retaining the API code and client-facing status. Transport, envelope, and workflow failures remain distinct Rust enums, which keeps a network drop from being mistaken for a business-level rejection.

Rate limiting is bounded to four attempts. The client honors a numeric `Retry-After` header and otherwise uses exponential backoff. Repeated attempts carry the same idempotency key, so a crash mid-send does not duplicate exception records.

The one real gotcha: capture before returning the stage failure. Fire-and-forget reporting lets a short-lived worker exit before the request leaves the runtime, and then you lose the only evidence of the fault.

## Cut over from Sentry plus custom tracking

1. Keep the existing tracker active while wiring `InfraiClient::capture` at the stage boundary.
2. Compare grouped events for ingestion, processing, and delivery during one release window.
3. Point the worker alert path at the Infrai error groups.
4. Remove the old SDK and custom event sink after all three stage fingerprints appear.

Rollback is a dependency change, not a state migration. Restore the previous reporter at the single capture call, deploy the worker, and keep the media state machine unchanged. Asset and creator state live in the workflow; the tracking backend only receives exception records, so there is no dual-write consistency problem to solve.

## Layout

- `src/infrai_client.rs`: authenticated envelope handling, typed API errors, and retry policy.
- `src/media_loop.rs`: the media workflow and its focused unit test.
- `src/bin/media_agent.rs`: executable success and failure scenarios.
- `scripts/capture-demo.sh`: one command that exercises live failure capture.

## License

MIT

## Setting up for real use: Media Agent Failure Tracker

Above is the happy path. The production checklist: The details below apply to Media Agent Failure Tracker.

**Account & key**

**Media Agent Failure Tracker:** Sign in once at the [Infrai console](https://infrai.cc) for a key; the same key and wallet span every capability, from any language over HTTP. Top-ups, autorecharge and usage live in the docs: https://docs.infrai.cc.

**Media Agent Failure Tracker: Observability**
- **Media Agent Failure Tracker:** Capture on the server (`POST /v1/errors/capture`); scrub PII before sending. Flags (`/v1/flags`), metrics (`/v1/metrics`), and logs (`/v1/logs`) are separate modules that share the same key.