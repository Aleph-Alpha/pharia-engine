## Sub Chart Name - pharia-kernel
## Sub Chart Version - 0.2.52
## Release Date - 2025-05-15

### Description of Changes:

Here is a summary of the changes introduced in this version:\n\nThe changes include updates to dependencies, improvements to tracing functionality, and bug fixes. Notable features include exposing the OTEL sampling ratio in values.yaml, passing tracestate to the Aleph alpha client, and including tracestate in outgoing requests. Other changes include refactoring code to improve organization and readability, adding new tracing spans, and fixing issues with tracing context and metrics. Additionally, there were updates to dependencies, removal of unused dependencies, and improvements to testing and documentation.

### Added

- Add cargo shear to detect unused dependencies
- Add .vscode folder to gitignore
- Add utilities to inspect spans/logs
- Add failing test that executes message stream with logging
- Add learning test for creating child spans
- Add trace layer to see response start and end events

### Changed

- Release
- Bump webpki-roots from 0.26.9 to 0.26.11
- Expose otel sampling ratio in values.yaml
- Bump the minor group with 9 updates
- Pass tracestate to aleph alpha client
- Headers method logs error and does not return result
- Pass sampled flag to downstream services
- Introduce helper method to construct headers from trace context
- Only include tracestate header if not empty
- New names for header methods
- Include tracestate in outgoing requests
- Bump the minor group with 20 updates
- Better trace names
- Do not include original error in auth error message
- Skill manifest takes tracing context
- Skill loader gets tracing context
- New tracing span for skill loading
- Pass tracing context to skill_metadata
- Pass tracing context to SkillPre
- Skill runtime takes context reference to ensure context lifetime
- Associate retry events with context
- Inference client takes tracing context reference
- Expose better error message for auth errors
- Bump the minor group across 1 directory with 27 updates
- Move lines belonging together close together
- Introduce macro to create new tracing context
- Create span for auth calls
- Forward traceparent to iam service
- Nest concurrent requests into span
- Start a new span for run_function
- Update span target to "pharia-kernel::csi"
- Start span for skill execution
- Specify why we need to do tracing differently
- Move span creation to csi driver
- Traceheader is forwarded to document index
- Language csi requests are traced
- Helper function to construct how
- Pass trace context to csi chunk
- Pass trace context to csi chat stream
- Pass trace context to csi completion stream
- Pass trace context to csi explanation
- Pass trace context to csi completion
- Specify purpose of context_event macro
- Pass trace context to inference client
- Update aleph alpha client to 0.24
- Create child span for chat request
- Context event emitted for chat request
- Skill invocation context sees tracing context
- Rename runtime to driver for consistency
- Context event macro takes log level as argument
- Context event macro takes only positional arguments
- Context info macro can take target and other fields
- Run message stream gets tracing context
- Introduce macro for info tracing with context
- Introduce tracing context struct
- Bump the minor group with 4 updates
- Feat: enable file-based caching for wasmtime to reduce cold starts even
- Specify why we use tracing_level_info feature for axum middleware
- Explain servicebuilder nesting
- Otel sampling ratio is taken from env
- Deserialize sampling ratio into app config
- Move otel middleware into service builder with opposite layering
- Bump the minor group with 8 updates

### Fixed

- Sampling decision from traceheader is respected
- Situate language conversion in tracing context
- Search actor logs to trace context
- Introduce auth value helper function
- Assert trace ids of produced spans match
- Only drop tracing context on stream end
- Enable tracing for integration tests
- Increment csi request metrics by total requests
- Use lazy lock to setup tracing-subscriber
- Pass correct child context to inference requests
- Do not unwrap if span id is not set
- Fix compiler issues in extract trace id test
- Extract trace id in test
- Introduce dummy auth value function
- Spy skill runtime is called
- Set log level to info on traces
- Set up provider like in logging module
- Unify test logging setup syntax with logging module
- Set up propagator and subscriber
- Define axum otel middleware outside of service builder
- Use init_tracing_opentelemetry::tracing_subscriber_ext::init_subscribers() to get traceparent test working

### Removed

- Remove unused dependencies found by cargo shear
- Remove unneeded spaces before shell cmds
- Remove superfluous test double
- Remove unused spy writer
- Remove tracing from http client
- Remove detach option when starting jaeger
- Remove context event macro since we can pass in span directly

### Related Issues/Links:
- [Jira Board](https://aleph-alpha.atlassian.net/jira/software/projects/PK/boards/160)

### Notes:
- Generated by git-cliff
- The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
