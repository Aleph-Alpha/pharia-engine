# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.15.0](https://github.com/Aleph-Alpha/pharia-engine/compare/pharia-engine-v0.14.0...pharia-engine-v0.15.0) (2025-10-22)


### ⚠ BREAKING CHANGES

* rename pharia-kernel to pharia-engine

### Features

* accumulate content if tracing for streaming chat ([7c36540](https://github.com/Aleph-Alpha/pharia-engine/commit/7c36540075cf2dc4f5af19ed848c6fe7e7d46d8f))
* add openai_client_v2 to inference AA client ([636a8b0](https://github.com/Aleph-Alpha/pharia-engine/commit/636a8b0427ff77321fc3c63bceadbbcc04db62f8))
* also support trace export via http ([775a4f7](https://github.com/Aleph-Alpha/pharia-engine/commit/775a4f7fabddb0611d742b0a960303d164b9a3c0))
* extract reasoning events in chat stream from AA inference API v1 ([ba819f7](https://github.com/Aleph-Alpha/pharia-engine/commit/ba819f707aca21ae9ed7a8bc28bb507dedbc62a0))
* Inject open_telemetry middleware in authorization requests ([8f7150b](https://github.com/Aleph-Alpha/pharia-engine/commit/8f7150b2c4a88098e7568b3f0f753a2bd46eef5c))
* introduce unstable 0.5 wit world ([87079a7](https://github.com/Aleph-Alpha/pharia-engine/commit/87079a703480cfb657f526a9fb561aefa28fe0ee))
* message stream event can emit reasoning content ([a836117](https://github.com/Aleph-Alpha/pharia-engine/commit/a8361176da2e1c17f2dee7106c7e7ab96d6723bb))
* optionally serialize reasoning_content from reasoning field ([7a16a7e](https://github.com/Aleph-Alpha/pharia-engine/commit/7a16a7e8261c79ce5666d5303350ede7ebe8c843))
* record input and output token usage on spans ([b4a9481](https://github.com/Aleph-Alpha/pharia-engine/commit/b4a94811a43a8a25bd61ccd4dc7a05fa73695bf0))
* remove logprobs from chat event v2 ([17657a1](https://github.com/Aleph-Alpha/pharia-engine/commit/17657a128d4f473ab24b2256b5427d93fc8e019e))
* stabilize 0.4 wit world ([99432aa](https://github.com/Aleph-Alpha/pharia-engine/commit/99432aae642d18442796fc888c7f243cbcd75c2a))
* store gen_ai attributes on csi spans ([5aa36ee](https://github.com/Aleph-Alpha/pharia-engine/commit/5aa36ee0fb0374ab998b9d26488d2d003814e970))


### Bug Fixes

* extract reasoning even if not closed ([244a146](https://github.com/Aleph-Alpha/pharia-engine/commit/244a1462f1bcb6b2dcc61607b654b27775ec273f))


### Code Refactoring

* rename pharia-kernel to pharia-engine ([1d8e139](https://github.com/Aleph-Alpha/pharia-engine/commit/1d8e1393cfbecb3589fe205d821a22b48dec7b26))

## [Unreleased]

## [0.14.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.24...pharia-kernel-v0.14.0)

### Features

- Increase http body limit to 50mb - ([b93bbab](https://github.com/Aleph-Alpha/pharia-kernel/commit/b93bbab1d098e91384d405cd8633c5ecdebacf38))
- Chunk text based on characters if tokenizer not available - ([8b23e44](https://github.com/Aleph-Alpha/pharia-kernel/commit/8b23e4400efaedbdb37c9bf8d11d0f7bc2731195))

### Fixes

- Error message when listing tools in non-existing namespace - ([56666a2](https://github.com/Aleph-Alpha/pharia-kernel/commit/56666a28312d12b8def186bdd0ef8d5cc5828f8c))

### Documentation

- Use sh code blocks consistently - ([177e0a4](https://github.com/Aleph-Alpha/pharia-kernel/commit/177e0a48629f1701550ec29ec35263c43512d5c6))

### Builds

- *(deps)* Bump actions/setup-python from 5 to 6 - ([6112314](https://github.com/Aleph-Alpha/pharia-kernel/commit/611231442c68869bd5916c34d5454f9253fa7e1e))
- *(deps)* Bump the minor group across 1 directory with 8 updates - ([31d13fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/31d13fd345a134dae533305d14775c7b983c9674))

### Refactor

- Make lifecycle managed by `main`. - ([409178b](https://github.com/Aleph-Alpha/pharia-kernel/commit/409178b15d502262633df0dc361c8f4cf31a67d3))

### Test

- Use dummies macro for SkillStoreApi - ([155ea21](https://github.com/Aleph-Alpha/pharia-kernel/commit/155ea21a1f6b475761ed7724435c265449abdf06))
- Use dummies for McpSubscriber - ([33bc511](https://github.com/Aleph-Alpha/pharia-kernel/commit/33bc5115d48647aa0419bd3c241de2a259680ee6))
- Use dummies macro for McpApi - ([395c552](https://github.com/Aleph-Alpha/pharia-kernel/commit/395c5523c931512940b81793399fadb503685d3c))
- Use dummies macro for ToolRuntimeApi - ([50614b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/50614b056c787da35aa70b25b94dc8841f0d19d7))
- ToolStoreApi is now doubled using dummies macro - ([592764d](https://github.com/Aleph-Alpha/pharia-kernel/commit/592764ddc12766bf04efd4e5438aa3171b3e6fe2))


## [0.13.24](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.23...pharia-kernel-v0.13.24)

### Fixes

- Map wasm assistant message to correct inference message variant - ([761b34c](https://github.com/Aleph-Alpha/pharia-kernel/commit/761b34c67389131bde8207478e8125098e93b4ed))


## [0.13.23](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.22...pharia-kernel-v0.13.23)

### Features

- Remove explain functionality in 0.4 wit world - ([57c3ec0](https://github.com/Aleph-Alpha/pharia-kernel/commit/57c3ec068dca1e9aa52ac81c231f14295825e53f))

### Builds

- *(deps)* Bump the minor group with 37 updates - ([3e64d2a](https://github.com/Aleph-Alpha/pharia-kernel/commit/3e64d2af40a2f4bacc57fef1b512788aeae77574))


## [0.13.22](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.21...pharia-kernel-v0.13.22)

### Features

- No need to fail on events without content or tool call - ([bc9c1c9](https://github.com/Aleph-Alpha/pharia-kernel/commit/bc9c1c9bfa72b4b7f73a6c3b9b3ccb81ceb7cecf))
- Return 401 instead of 400 when no token is provided - ([1615a76](https://github.com/Aleph-Alpha/pharia-kernel/commit/1615a76853aad85ab65c8b045531ec97bd9f17aa))

### Fixes

- Allow empty choice stream chunk to be compatible with GitHub models - ([4ea4769](https://github.com/Aleph-Alpha/pharia-kernel/commit/4ea47698bb4182c08520f9456954acc3b146cb39))
- Tool call events can contain role and content - ([4cdc293](https://github.com/Aleph-Alpha/pharia-kernel/commit/4cdc29331736b6519921460a288d1003583901fb))
- Remove unneeded since gates on 0.4 wit world - ([fbaa64c](https://github.com/Aleph-Alpha/pharia-kernel/commit/fbaa64c5496d580c7ae9ffc2cdd4c563e4083186))

### Documentation

- Specify cla is not yet in place - ([d50a50d](https://github.com/Aleph-Alpha/pharia-kernel/commit/d50a50d88cb7a9c62eb5d51be4eb3524bc1f9168))
- Add contributing.md - ([f20b91c](https://github.com/Aleph-Alpha/pharia-kernel/commit/f20b91c6a938202304f5549adf054ace67546023))
- Add LICENSE.md - ([8f9d229](https://github.com/Aleph-Alpha/pharia-kernel/commit/8f9d229ff288064a075d43beb9433d13ff3244d1))

### Refactor

- From stream returns vec of events - ([c449e94](https://github.com/Aleph-Alpha/pharia-kernel/commit/c449e94f2e86a60a6375019d7c1aa9e70ce529ad))
- Use normal impl block for event conversion - ([bd36049](https://github.com/Aleph-Alpha/pharia-kernel/commit/bd36049702e27f430d201212e51eaaf4a0838672))


## [0.13.21](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.20...pharia-kernel-v0.13.21)

### Features

- Make base_repository optional - ([aa943a4](https://github.com/Aleph-Alpha/pharia-kernel/commit/aa943a4b871edd56d4daf732e7c98894d16e4fa0))
- Support Minimal variant of ReasoningEffort - ([67a16fc](https://github.com/Aleph-Alpha/pharia-kernel/commit/67a16fc49d9c9fe9128587f024a7dbacc5b0dbe5))
- User decides between max-tokens and max-completion-tokens - ([7fa92ff](https://github.com/Aleph-Alpha/pharia-kernel/commit/7fa92ffa8eec3f11cafcc9b1070c642723165f0e))
- Include reasoning-effort in chat-params - ([e7a806e](https://github.com/Aleph-Alpha/pharia-kernel/commit/e7a806e613ae3d038af70a71420150aa65559ea9))
- Expose 0.4 tool call params via csi shell - ([0d6f471](https://github.com/Aleph-Alpha/pharia-kernel/commit/0d6f471055a04ebcdb2fbd81c270248cb44d4fd8))
- Add support for structured output - ([837a0e0](https://github.com/Aleph-Alpha/pharia-kernel/commit/837a0e0f9033f2b18bad5f40fe26d2ceae660ace))
- Support tool call stream events in 0.4 wit world - ([bbe7118](https://github.com/Aleph-Alpha/pharia-kernel/commit/bbe71186150f3501d4e5b33089e7e328bfe43bbe))
- Support parallel-tool-call parameter in 0.4 wit world - ([8784cc7](https://github.com/Aleph-Alpha/pharia-kernel/commit/8784cc71c967d07539763efa7294d523de295141))
- Support tool-choice parameter in 0.4 wit world - ([50d0505](https://github.com/Aleph-Alpha/pharia-kernel/commit/50d050577479e09381a4209f20ce40f776dd77c3))
- Propagate tool call to 0.4 wit world - ([ea36ac0](https://github.com/Aleph-Alpha/pharia-kernel/commit/ea36ac0f3928a8068d782fb4615fc58409db0d55))

### Fixes

- Set max_completion_tokens for reasoning models instead of max_tokens - ([b638a54](https://github.com/Aleph-Alpha/pharia-kernel/commit/b638a540f3b665cf1fecfccf21b52dc356bc80a7))

### Builds

- *(deps)* Bump actions/checkout from 4 to 5 - ([e75cd13](https://github.com/Aleph-Alpha/pharia-kernel/commit/e75cd13cd67b6827b10be19416140ad1793bedda))
- *(deps)* Bump actions/download-artifact from 4 to 5 - ([32ece28](https://github.com/Aleph-Alpha/pharia-kernel/commit/32ece28854fa71b9429dcb2279b54ccda0d530ef))
- *(deps)* Bump the minor group across 1 directory with 7 updates - ([af90a29](https://github.com/Aleph-Alpha/pharia-kernel/commit/af90a29cb399b94fcc4d27116d7c6c3b10640d37))

### Chore

- Add skill build cache to .gitignore - ([d6c99d1](https://github.com/Aleph-Alpha/pharia-kernel/commit/d6c99d11839452dc6f8031839990f5569cea9e68))

### Ci

- Update pinned version of trivy action - ([269155b](https://github.com/Aleph-Alpha/pharia-kernel/commit/269155b6b850f29e02aee5cbad35c1827eec1617))

### Refactor

- Panic in csi if receiving tool call for early wit worlds - ([054e5cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/054e5cb9f3e95029330cafa0b1f17098ddc35e8e))
- Represent message as enum internally - ([9c19d26](https://github.com/Aleph-Alpha/pharia-kernel/commit/9c19d26be72fc804cb7637993537650a8cc8a778))
- Use async_openai for chat requests against AA inference - ([1595e9b](https://github.com/Aleph-Alpha/pharia-kernel/commit/1595e9bf75895feb62636f79b0cdea71aea0bfcc))
- Rename internal client to aleph_alpha - ([6185ee8](https://github.com/Aleph-Alpha/pharia-kernel/commit/6185ee87c2ca74ae766134930a445036b20e1dfa))
- Introduce aleph alpha client struct - ([2da7ab3](https://github.com/Aleph-Alpha/pharia-kernel/commit/2da7ab3214438caa49af59fee51acb1c37a288b1))
- Check precondition on inference responses in client - ([4b47f97](https://github.com/Aleph-Alpha/pharia-kernel/commit/4b47f979f36f00874d8f83d299bbbc528abeab86))
- Make response message content optional - ([6876287](https://github.com/Aleph-Alpha/pharia-kernel/commit/68762875b1826340d38de2b8873c7d678969e565))
- Split Message from ResponseMessage - ([1926b4a](https://github.com/Aleph-Alpha/pharia-kernel/commit/1926b4a06a9a71eab9c8cf04170ed6df14d770d7))
- Introduce tool call variant on finish reason - ([d9540a6](https://github.com/Aleph-Alpha/pharia-kernel/commit/d9540a6e518b8dd0fdf0f4279ebf10852081fc1a))
- Introduce tools parameter 0.4 wit world - ([7c4c369](https://github.com/Aleph-Alpha/pharia-kernel/commit/7c4c369bc36582d321870a0e3f59706ba137eba1))

### Style

- Apply 1.89 lints - ([c5afdd6](https://github.com/Aleph-Alpha/pharia-kernel/commit/c5afdd6f770c02317f4bee84d7e1f76cef5a2a77))

### Test

- Pull out model names into consts - ([0fc2153](https://github.com/Aleph-Alpha/pharia-kernel/commit/0fc2153459e94aba859c4a45e3f5a4192547996f))
- Add test_no_openai feature - ([e342952](https://github.com/Aleph-Alpha/pharia-kernel/commit/e342952fb313a2ba29a3e044d494146720d54b03))
- Clear skill build cache on wit changes - ([e60bfd1](https://github.com/Aleph-Alpha/pharia-kernel/commit/e60bfd1b18f17788dcbea76218ad2a906694bbe9))


## [0.13.20](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.19...pharia-kernel-v0.13.20)

### Features

- Use completion-params-v2 for completion-request - ([baea817](https://github.com/Aleph-Alpha/pharia-kernel/commit/baea817c146763ce47a79ac204ab2bc568105f82))
- Mark 0.4 wit world as unstable - ([553e139](https://github.com/Aleph-Alpha/pharia-kernel/commit/553e139a253e0480f7ae1da844fa2cbc0e4ffaf6))
- Introduce 0.4 wit world - ([8aec913](https://github.com/Aleph-Alpha/pharia-kernel/commit/8aec913d22a318d5139dd7dc13223c539cc7792f))
- Do not specify a namespace in default config.toml - ([b6105c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/b6105c8ee42c74b480082fa676c2bbb7274dd34d))
- Allow running without requiring bearer token - ([5ab0e9b](https://github.com/Aleph-Alpha/pharia-kernel/commit/5ab0e9b45b36bc6e789a3465ba8b3c761a432b26))
- Authorization client decides if no token is okay - ([75c900a](https://github.com/Aleph-Alpha/pharia-kernel/commit/75c900a909f8a1293c573aef92ddceee7b6aca7f))
- Make authorization check optional - ([bbf7f08](https://github.com/Aleph-Alpha/pharia-kernel/commit/bbf7f08cf5cf85361284c14bbb2da9bd13daa678))

### Fixes

- Remove unused allow dead code - ([a3495d2](https://github.com/Aleph-Alpha/pharia-kernel/commit/a3495d2be76f14d958c2af5d0a5e22f6014f1974))

### Documentation

- Streamline formulations - ([a3b4414](https://github.com/Aleph-Alpha/pharia-kernel/commit/a3b4414d89d35fca053282d54df97c54a41b89e4))
- Split .env example and test example - ([0ebbe36](https://github.com/Aleph-Alpha/pharia-kernel/commit/0ebbe3682650ce798d8b38c593a13f35b8083fa8))
- Reorder README sections - ([3dde600](https://github.com/Aleph-Alpha/pharia-kernel/commit/3dde60049c633e5f70ca9b484bcf03bb88bd806e))
- Fix some typos & small lints - ([59b7965](https://github.com/Aleph-Alpha/pharia-kernel/commit/59b796552b5379dcfe38024dabe152cbaaf610e5))
- Specify what happens if auth url is none - ([70b3a43](https://github.com/Aleph-Alpha/pharia-kernel/commit/70b3a43d6f8dd80e9ebc845074557b04e4fa1095))

### Builds

- *(deps)* Bump the minor group across 1 directory with 13 updates - ([df13a42](https://github.com/Aleph-Alpha/pharia-kernel/commit/df13a429e63f020cea064131528afebc384a71a6))
- *(deps)* Bump the minor group across 1 directory with 8 updates - ([16b4938](https://github.com/Aleph-Alpha/pharia-kernel/commit/16b49384423dad85b36a25fa04cbcc431dd30097))
- *(deps)* Bump the minor group across 1 directory with 9 updates - ([d3ff1d7](https://github.com/Aleph-Alpha/pharia-kernel/commit/d3ff1d73983524949023861d858cbdfa8fdbe651))

### Chore

- Load test envs from .env.test - ([a2748f2](https://github.com/Aleph-Alpha/pharia-kernel/commit/a2748f2dc6c15a0d82437ec79891107c07f96208))

### Ci

- Push to Harbor - ([4791fbf](https://github.com/Aleph-Alpha/pharia-kernel/commit/4791fbfdcc043051eec7ca1e501bc55731347e8d))
- Pin Red Hat GitHub actions - ([41ab6d9](https://github.com/Aleph-Alpha/pharia-kernel/commit/41ab6d9d7ce0b8f73a55e27631abee3d691654e3))
- Use internal actions - ([4b37eff](https://github.com/Aleph-Alpha/pharia-kernel/commit/4b37effcfb83f95f2232c0a9e45e9fd3ec68267a))
- Use CycloneDX Cargo action - ([ac21d3c](https://github.com/Aleph-Alpha/pharia-kernel/commit/ac21d3ce2fcb441b38800a7dbd1edbe5759818c0))
- Merge all CycloneDX files - ([e0965e5](https://github.com/Aleph-Alpha/pharia-kernel/commit/e0965e5f9cca7f18820bddb67f6a4d5bad4d28d7))
- Merge SBOM with licenses - ([079d285](https://github.com/Aleph-Alpha/pharia-kernel/commit/079d2857ec82f1cba980fbd69be4c5733d2cf56d))
- Add license scanner - ([13c3d61](https://github.com/Aleph-Alpha/pharia-kernel/commit/13c3d61c2ff6542bebacf94a267c0faaeda561af))
- Set PhariaAI services for helm-test in values-kind.yaml - ([f449c4e](https://github.com/Aleph-Alpha/pharia-kernel/commit/f449c4e8584d85f00008ea32bc45c109c798c00a))
- Do not provide unneeded envs and config to image integration test - ([ad3805f](https://github.com/Aleph-Alpha/pharia-kernel/commit/ad3805f8868de18db7e00338b8ea23f10b906323))

### Refactor

- Use wit-parser source map to assemble package - ([fc31b33](https://github.com/Aleph-Alpha/pharia-kernel/commit/fc31b33b1aa138fa7f894921a4d3ddc5fea62516))
- Rename next-release feature gate to alpha - ([e615410](https://github.com/Aleph-Alpha/pharia-kernel/commit/e615410ec8ee16dd880a66054eab17b1d0545d38))
- Remove completion-request-v2 - ([847fa37](https://github.com/Aleph-Alpha/pharia-kernel/commit/847fa37cb9f188c138ea5f1119b8b413b648fea0))
- Remove v2 suffix from completion-params - ([aa70880](https://github.com/Aleph-Alpha/pharia-kernel/commit/aa708808f874eceb85034c7cfd42a0b0def4594a))
- Authentication wraps an option - ([52e2ec5](https://github.com/Aleph-Alpha/pharia-kernel/commit/52e2ec5e1c7f65a58dd848bfcfae8933e5478a49))
- Introduce authentication new type - ([df8abb0](https://github.com/Aleph-Alpha/pharia-kernel/commit/df8abb0945cd1212b8d2c2e90ef1a47a13dafe55))
- Introduce new type authentication - ([952c735](https://github.com/Aleph-Alpha/pharia-kernel/commit/952c735f09303101592cbe83b9cc85832985e8f8))

### Test

- TestKernel owns tracing subscriber guard - ([6355bca](https://github.com/Aleph-Alpha/pharia-kernel/commit/6355bca11e56a88b642577e151ab30513641252f))
- Fix flaky tracing tests - ([711504f](https://github.com/Aleph-Alpha/pharia-kernel/commit/711504f85e5cd9f7edbcee54d878c44ad2c5c4f7))
- Update registry names to not overlap with namespace configuration - ([be48b4f](https://github.com/Aleph-Alpha/pharia-kernel/commit/be48b4f6756e8d4c440fb655c7c4d5320bd8cbff))
- Remove test as reading from config file is already covered - ([8a0f4c0](https://github.com/Aleph-Alpha/pharia-kernel/commit/8a0f4c071e14c181d741e5eff709daba52d17d82))
- Remove unneeded auth token - ([388f2b9](https://github.com/Aleph-Alpha/pharia-kernel/commit/388f2b9baaa77fe30f5a95235a4528e7064bc4a3))
- Skill execution against openai - ([5ddc2c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/5ddc2c8742039673a4374248d818cc5ef25a79e5))
- Replace dummy function with none - ([94b56d5](https://github.com/Aleph-Alpha/pharia-kernel/commit/94b56d5aecc9b19e8c8fc94306294f5161e71fcb))
- Auth actor without url always returns true - ([25c4e0c](https://github.com/Aleph-Alpha/pharia-kernel/commit/25c4e0ca33e9fe60053906463f5b4785e3f75aea))
- Restructure utilities for test kernel - ([31c6830](https://github.com/Aleph-Alpha/pharia-kernel/commit/31c68306ac218e84d7eeec99a99084d486032c02))


## [0.13.19](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.18...pharia-kernel-v0.13.19)

### Features

- Migrate to check_user iam endpoint - ([33697f3](https://github.com/Aleph-Alpha/pharia-kernel/commit/33697f389c9a82b9931e6c3ae51f70284d90b19d))
- Implement openai client - ([40cf128](https://github.com/Aleph-Alpha/pharia-kernel/commit/40cf1288cbac985a7c93fcef375d6fd263e2d7c8))
- OpenAI inference backend can be configured - ([e99d410](https://github.com/Aleph-Alpha/pharia-kernel/commit/e99d4108236bd92b985c24d77e791aacd26f9fd5))
- Add warning if running without inference capabilities - ([d9061cc](https://github.com/Aleph-Alpha/pharia-kernel/commit/d9061cc85d0d58218c5b802fd89e348e97d32586))
- Inference url is optional in app config - ([a62f135](https://github.com/Aleph-Alpha/pharia-kernel/commit/a62f135dee63b5b48c6f0f1c1f3b9b6c19ed7574))
- Map empty document index url to none in app config - ([858df5c](https://github.com/Aleph-Alpha/pharia-kernel/commit/858df5cb450795874523d129770d6bb68829ae74))
- Document index is an optional dependency - ([8a7ea86](https://github.com/Aleph-Alpha/pharia-kernel/commit/8a7ea8629b76047a0bc0878e084ee58009e6a725))
- Custom logs without span context and attributes - ([268cd96](https://github.com/Aleph-Alpha/pharia-kernel/commit/268cd962a3148b16d84a359a5c315f3444ec07c7))

### Builds

- *(deps)* Bump the minor group across 1 directory with 16 updates - ([cbf89af](https://github.com/Aleph-Alpha/pharia-kernel/commit/cbf89afc0ff68f9eec3e54ce9f3b6257c29ca304))
- *(deps)* Bump inference client for sse bug fix - ([57d8368](https://github.com/Aleph-Alpha/pharia-kernel/commit/57d8368c4770b47aa70e635a6fb1969e01f6e6c0))


## [0.13.18](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.17...pharia-kernel-v0.13.18)

### Features

- Include error in logs when skill execution fails - ([1d6e7cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/1d6e7cbe850c55b2e6626423f67dcd34eafd0f71))
- Log skill loading errors - ([d9c70ca](https://github.com/Aleph-Alpha/pharia-kernel/commit/d9c70ca464667d5753263194f4264e7ab0d4e0a8))
- Better error msg for 404 mcp server result - ([b23ff38](https://github.com/Aleph-Alpha/pharia-kernel/commit/b23ff38d2d047dd381a0641c8cb5791b1396cb35))

### Fixes

- Notify tool box about all namespaces - ([a722280](https://github.com/Aleph-Alpha/pharia-kernel/commit/a7222802381cbf1685b1d056177d09c8fd7a277c))
- Tool box can report error if namespace does not exist - ([5119e8f](https://github.com/Aleph-Alpha/pharia-kernel/commit/5119e8f536ed5cb240976d6502892fb1d70bb75d))
- Return 404 when querying mcp-server list for non-existing namespace - ([ea6f119](https://github.com/Aleph-Alpha/pharia-kernel/commit/ea6f119e0227ce157cf8e8b339d631e8cf4050e0))

### Builds

- *(deps)* Bump the minor group across 1 directory with 9 updates - ([134f6f1](https://github.com/Aleph-Alpha/pharia-kernel/commit/134f6f12da92bcbc8765d142f22b9cb2ab2a28f4))
- *(deps)* Bump the minor group across 1 directory with 4 updates - ([0f11a71](https://github.com/Aleph-Alpha/pharia-kernel/commit/0f11a7173df7a25678353522f3836b8c876a94e7))


## [0.13.16](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.15...pharia-kernel-v0.13.16)

### Features

- Order mcp servers by name when listing - ([943dc4d](https://github.com/Aleph-Alpha/pharia-kernel/commit/943dc4d9bd8258e23ea3fe0d8a1ea36947853ba7))

### Fixes

- Report updated tools if existing mcp server is configured for new namespace - ([7e8c481](https://github.com/Aleph-Alpha/pharia-kernel/commit/7e8c4811d709505b2e1a179e2be8a57ddf0475d1))

### Builds

- *(deps)* Bump the minor group with 2 updates - ([7c7ecc6](https://github.com/Aleph-Alpha/pharia-kernel/commit/7c7ecc69e9c8dadc2050c111e351e10ed8b89caf))
- *(deps)* Bump aquasecurity/trivy-action from 0.31.0 to 0.32.0 - ([13adb4a](https://github.com/Aleph-Alpha/pharia-kernel/commit/13adb4aa5f253abd07c54e3ee68ba8bc7af9f4e0))
- *(deps)* Update to wasmtime 0.33.1 - ([e8e8fbb](https://github.com/Aleph-Alpha/pharia-kernel/commit/e8e8fbb0ccec91bbc03ff2670d78c867fc233abb))


## [0.13.15](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.14...pharia-kernel-v0.13.15)

### Features

- Remove content from tool stream event - ([d530f7f](https://github.com/Aleph-Alpha/pharia-kernel/commit/d530f7fad75486dacd75b4ded44b2bec958920d1))
- Truncate large tool responses in message stream route - ([0cbeec2](https://github.com/Aleph-Alpha/pharia-kernel/commit/0cbeec250fca281773390f976813e248c57c3f00))
- Report tool output in stream - ([b118c9c](https://github.com/Aleph-Alpha/pharia-kernel/commit/b118c9c2bdddca3c6a2279890151da33a4ac2793))
- Expose tool invocation errors in message stream - ([c907e28](https://github.com/Aleph-Alpha/pharia-kernel/commit/c907e284cc2060205771bab5e0c1be4aa19ecc7d))
- Tool end events are exposed via routes - ([22bec69](https://github.com/Aleph-Alpha/pharia-kernel/commit/22bec696f2771af2865ca93b890b42cb055f30bc))
- Tool begin event serialized via routes - ([b22d354](https://github.com/Aleph-Alpha/pharia-kernel/commit/b22d354e9d028513a3e3ab2f8e99021d789091cb))
- Skill driver forwards skill ctx event - ([9ea981e](https://github.com/Aleph-Alpha/pharia-kernel/commit/9ea981e6f6256311c8e2408b35b87f728d7e2e0e))
- Skill context emits tool call event - ([46e3c49](https://github.com/Aleph-Alpha/pharia-kernel/commit/46e3c49069d2a09b1566219857642378c411a266))
- Add tool call hardcoded skill - ([3798d13](https://github.com/Aleph-Alpha/pharia-kernel/commit/3798d13f667db2daceadb653b6f55283b4771566))

### Fixes

- Tool caller sends arguments as json integers - ([16178bb](https://github.com/Aleph-Alpha/pharia-kernel/commit/16178bb78fc682ff72c8000275dd01e06b85bf04))

### Documentation

- Add tool caller to example responses - ([bd9d4c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/bd9d4c893c72c0b13148fa8d249305806a68e9b8))

### Builds

- *(deps)* Bump the minor group across 1 directory with 7 updates - ([33224d5](https://github.com/Aleph-Alpha/pharia-kernel/commit/33224d52e5241b31d6e6eeb6f3808b2a23f5903e))


## [0.13.14](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.13...pharia-kernel-v0.13.14)

### Documentation

- Fix typo - ([9daa1d8](https://github.com/Aleph-Alpha/pharia-kernel/commit/9daa1d82d63c96054953fa5b59a4ec7272f1bcb4))


## [0.13.13](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.12...pharia-kernel-v0.13.13)

### Builds

- *(deps)* Bump the minor group across 1 directory with 7 updates - ([bfcadf4](https://github.com/Aleph-Alpha/pharia-kernel/commit/bfcadf49f6c76b87b099508edc337c2d358382d1))


## [0.13.12](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.11...pharia-kernel-v0.13.12)

### Features

- Promote list_mcp_servers and list_tools routes - ([54311ba](https://github.com/Aleph-Alpha/pharia-kernel/commit/54311ba53ca97a47ede73c3b8d99ea53ee34803e))
- Better error message on model not found - ([56d2de2](https://github.com/Aleph-Alpha/pharia-kernel/commit/56d2de214b08a0831afb86d4bcb2bbe547aea9e8))


## [0.13.10](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.9...pharia-kernel-v0.13.10)

### Features

- Stablize tool interface as wit package version v0.3.11 - ([0ae6f54](https://github.com/Aleph-Alpha/pharia-kernel/commit/0ae6f54c9d8c210278f535111bf444f305fd4aec))
- Include environment variable name in error message - ([97ea9fa](https://github.com/Aleph-Alpha/pharia-kernel/commit/97ea9faebfe2c6aff5a94605fd2b67c8563e0c75))
- Log error if 401/403 when accessing registry - ([0a065d2](https://github.com/Aleph-Alpha/pharia-kernel/commit/0a065d280e2b12d8fd655262eddcb7606be5de03))


## [0.13.9](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.8...pharia-kernel-v0.13.9)

### Documentation

- Fix typos - ([d953e2b](https://github.com/Aleph-Alpha/pharia-kernel/commit/d953e2bea263a65c5247fc2c59f2d1dae17e0f86))


## [0.13.8](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.7...pharia-kernel-v0.13.8)

### Features

- Expose list tools via csi shell - ([72d5736](https://github.com/Aleph-Alpha/pharia-kernel/commit/72d573636f4defa7fd18bca1378d8c2c655c93b7))
- Expose list tools in wit world - ([2e26361](https://github.com/Aleph-Alpha/pharia-kernel/commit/2e263612d15946a2035cd2bde88f020005cb01b5))

### Fixes

- Bump wit version which was missed before - ([8a6dd06](https://github.com/Aleph-Alpha/pharia-kernel/commit/8a6dd06732b755c3208534dad4ac31cd2a4f3099))

### Documentation

- Fix typo - ([f1c37b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/f1c37b064c5edff6c1167bb5e24bc26d97d4a79d))


## [0.13.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.5...pharia-kernel-v0.13.6)

### Features

- Offer tool description via tool routes - ([d77d20f](https://github.com/Aleph-Alpha/pharia-kernel/commit/d77d20f8cc90f7a37bba0755618909727336c77a))

### Builds

- *(deps)* Bump the minor group with 16 updates - ([f3c8113](https://github.com/Aleph-Alpha/pharia-kernel/commit/f3c8113fc1fd66351e3da07606e21bae70bea3f7))
- *(deps)* Bump the minor group across 1 directory with 11 updates - ([e737211](https://github.com/Aleph-Alpha/pharia-kernel/commit/e7372114d10797e8b8335775a069b3d6d55b01b9))


## [0.13.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.4...pharia-kernel-v0.13.5)

### Features

- Introduce native tool saboteur - ([5cd0cf6](https://github.com/Aleph-Alpha/pharia-kernel/commit/5cd0cf6f0b35ac92aac75ff143a5657ad95ca5b0))
- Tool invocation errors are part of csi shell body - ([32e9cc3](https://github.com/Aleph-Alpha/pharia-kernel/commit/32e9cc3053d16fbc89bd72815ce93c2861f4e39d))
- Pass tool errors to skills - ([9edb767](https://github.com/Aleph-Alpha/pharia-kernel/commit/9edb7677e7ba290fd7ce514c8a9f415c1b74a954))
- Expose tool errors in the wit world - ([8285a9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/8285a9a58dbf24f2dc1c16a7f60507ea65d7548c))


## [0.13.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.3...pharia-kernel-v0.13.4)

### Documentation

- Fix list_mcp_servers response example - ([c5376f8](https://github.com/Aleph-Alpha/pharia-kernel/commit/c5376f85b252b367d80b2f0985432593abdf32b0))

### Builds

- *(deps)* Bump the minor group across 1 directory with 5 updates - ([b856d61](https://github.com/Aleph-Alpha/pharia-kernel/commit/b856d610c6fb6a8e377ff2c1a4a613d95a940257))


## [0.13.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.2...pharia-kernel-v0.13.3)

### Features

- List hardcoded native tools in test-beta namespace - ([df7ef5f](https://github.com/Aleph-Alpha/pharia-kernel/commit/df7ef5f33e2faf90c623f0b2d029038c1e266a5b))
- Implement invoke for native tool - ([4b63024](https://github.com/Aleph-Alpha/pharia-kernel/commit/4b6302467d5e5fb46f87eef9d69089e1b4c4f6e2))
- Fetch native tools - ([f82d0b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/f82d0b00660dd345f040fab1cb7d8f84f42044d0))
- List native tools - ([a5737c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/a5737c80963e7dcdd43fe008bf9255eb61e843a9))

### Fixes

- Native tools and mcp servers are configured in kebab-case - ([95bc0af](https://github.com/Aleph-Alpha/pharia-kernel/commit/95bc0afd611140f0150e47c1bfa6c549bc7e9ed8))

### Documentation

- Update description of skills route - ([db4c7c0](https://github.com/Aleph-Alpha/pharia-kernel/commit/db4c7c04ccf762a42caf01d9e5b24e38c64d6c9f))
- Fix missing skills route for non-beta OpenAPI docs - ([33d0cbe](https://github.com/Aleph-Alpha/pharia-kernel/commit/33d0cbe263a9ce5ba5c68912e14f57c4d6522c98))

### Builds

- *(deps)* Bump the minor group with 8 updates - ([2071682](https://github.com/Aleph-Alpha/pharia-kernel/commit/207168220d9478b3981937d7de88cfa90a65e5bb))


## [0.13.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.1...pharia-kernel-v0.13.2)

### Fixes

- Server sent events can span multiple http chunks - ([aedec1c](https://github.com/Aleph-Alpha/pharia-kernel/commit/aedec1c8671efa30494fa184d6824b5d497b694a))


## [0.13.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.13.0...pharia-kernel-v0.13.1)

### Features

- Respect OS certificate store for outgoing http requests - ([d3b757b](https://github.com/Aleph-Alpha/pharia-kernel/commit/d3b757b15e92be0dc192c4f29909065eaf9cebc3))

### Documentation

- Explain why we use `rust-tls-native-roots` - ([357d862](https://github.com/Aleph-Alpha/pharia-kernel/commit/357d862beab206daaaa452553a465e265030184e))


## [0.13.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.17...pharia-kernel-v0.13.0)

### Features

- Add target and context to error messages - ([745b1e4](https://github.com/Aleph-Alpha/pharia-kernel/commit/745b1e4de144a26249faa790c81b72d3e4297104))
- Improve error logging - ([d5e7e79](https://github.com/Aleph-Alpha/pharia-kernel/commit/d5e7e79d64e69c594812c8ec0b94e5678f27230f))
- Mcp Actor keeps an up to date tool list and informs tool runtime - ([bac24e8](https://github.com/Aleph-Alpha/pharia-kernel/commit/bac24e8a65e31f30ce21cd961ebbb193d52ef7f0))
- Print causes for error fetching namespace description - ([4e67ee7](https://github.com/Aleph-Alpha/pharia-kernel/commit/4e67ee761c3d0a949f09fe2320674f876e3427ce))
- Allow configuring native_tools, no computing of diffs yet - ([a158e3a](https://github.com/Aleph-Alpha/pharia-kernel/commit/a158e3a45505824753a2adc8558e542a8c27ccb2))
- Remove beta flag from configuration parsing - ([1426390](https://github.com/Aleph-Alpha/pharia-kernel/commit/142639035fed58156c9c5fffa7a7387b5f264804))
- Log invalid input - ([b01ecb0](https://github.com/Aleph-Alpha/pharia-kernel/commit/b01ecb029290c5bcdbfebdc125e04db2cb773b30))

### Fixes

- Only schedule special tool update if server is new - ([88fcb6a](https://github.com/Aleph-Alpha/pharia-kernel/commit/88fcb6ab4f46e2ad4dc595a53fbfd2b87e54143b))
- Do not block message queue of mcp actor - ([f8ee81f](https://github.com/Aleph-Alpha/pharia-kernel/commit/f8ee81fe9521f35a475b5f4e98da030e46f96169))

### Documentation

- Specify purpose of tool actor module - ([76c12df](https://github.com/Aleph-Alpha/pharia-kernel/commit/76c12df8860564bdb0913578203f5235b96f0ef3))
- Fix doc string on ToolStoreApi - ([aba9fca](https://github.com/Aleph-Alpha/pharia-kernel/commit/aba9fca9bf78499981ea9f31bf7e3f89a4eaa9f9))
- Add module docstring for mcp actor - ([9e06391](https://github.com/Aleph-Alpha/pharia-kernel/commit/9e063912d10ce41cf8fb79b86b63dc8c5f418a6b))
- Update docstrings and fix typos - ([ffdfe40](https://github.com/Aleph-Alpha/pharia-kernel/commit/ffdfe4037dd3507957d77189260d96a70357d7c2))
- Update docstring for tool trait and move to top of module - ([337f6a4](https://github.com/Aleph-Alpha/pharia-kernel/commit/337f6a46ad274e85bf4e77d53db9a0c44849265f))
- Update comment for native tools - ([c45a1c2](https://github.com/Aleph-Alpha/pharia-kernel/commit/c45a1c2d4dc465e2ddd79bc14529c07a90198e39))

### Performance

- [**breaking**] Cached mcp servers are used in tool invocations - ([55b728b](https://github.com/Aleph-Alpha/pharia-kernel/commit/55b728b909824bc2f422f62b11be16152ea20626))

### Builds

- *(deps)* Bump the minor group with 15 updates - ([0248efc](https://github.com/Aleph-Alpha/pharia-kernel/commit/0248efc4977cc5a72497a7ff0e92f19a04ae9578))
- *(deps)* Bump the minor group across 1 directory with 12 updates - ([cb414c1](https://github.com/Aleph-Alpha/pharia-kernel/commit/cb414c11d3ca40114828d7af7bf2eb9597272306))


## [0.12.17](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.16...pharia-kernel-v0.12.17)

### Features

- Expose invoke tool via csi shell - ([13a7baf](https://github.com/Aleph-Alpha/pharia-kernel/commit/13a7bafdbb3f62e5bbe8c08e420274f84978d3e1))


## [0.12.16](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.15...pharia-kernel-v0.12.16)

### Features

- Allow empty list as tool call result - ([c3ee9ae](https://github.com/Aleph-Alpha/pharia-kernel/commit/c3ee9aebb3bb26cdd72f9af48b06e1fa09327be1))

### Fixes

- Represent tool result as vec of modalities - ([5671d9c](https://github.com/Aleph-Alpha/pharia-kernel/commit/5671d9cc7fb826038506c26b5f985d749bf492c2))


## [0.12.15](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.14...pharia-kernel-v0.12.15)

### Features

- Add route to list available tools - ([01896b7](https://github.com/Aleph-Alpha/pharia-kernel/commit/01896b77e9da42d59f03dd70b4527e5e6a075226))


## [0.12.14](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.13...pharia-kernel-v0.12.14)

### Fixes

- Bump aa client to fix echo param with logprobs - ([ac12c40](https://github.com/Aleph-Alpha/pharia-kernel/commit/ac12c40c5e1b23b2d52ab66c074798fd752e9bac))
- Openapi.json now returns specification according to feature set - ([a795dd6](https://github.com/Aleph-Alpha/pharia-kernel/commit/a795dd6c5f5c9821d550752642488df01c37793e))

### Builds

- *(deps)* Bump remaining otel crates - ([3acd832](https://github.com/Aleph-Alpha/pharia-kernel/commit/3acd8329e4e5ce8661bca97339cf728405f9932b))
- *(deps)* Bump the minor group across 1 directory with 46 updates - ([39dc579](https://github.com/Aleph-Alpha/pharia-kernel/commit/39dc57963516e8f831026464e2e847091ec07216))


## [0.12.13](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.12...pharia-kernel-v0.12.13)

### Features

- Use actual app state to list MCP servers, test outstanding. - ([653a154](https://github.com/Aleph-Alpha/pharia-kernel/commit/653a15498db640a018a45b3ff221313f3b3739dd))
- Register route for listing mcp servers as beta - ([8c0bb32](https://github.com/Aleph-Alpha/pharia-kernel/commit/8c0bb32f54261eb895d81938bff0ad1ec9dc217e))

### Builds

- *(deps)* Replace double_derive with double_trait - ([57cf4be](https://github.com/Aleph-Alpha/pharia-kernel/commit/57cf4bef02a7656176d0d64c19d83c4bb30825b7))
- *(deps)* Bump aquasecurity/trivy-action from 0.30.0 to 0.31.0 - ([810d45e](https://github.com/Aleph-Alpha/pharia-kernel/commit/810d45e68e093e98e01fea470223f06fbde4449b))


## [0.12.12](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.11...pharia-kernel-v0.12.12)

### Features

- Only mcp servers configured for the skill namespace are accessed - ([b33c98a](https://github.com/Aleph-Alpha/pharia-kernel/commit/b33c98a7d751a91039b5b59a4190257142fe2f52))
- Add basic tracing to tool calls - ([51a539f](https://github.com/Aleph-Alpha/pharia-kernel/commit/51a539f201c13349dc330932032e81b715e41054))
- Mcp client can list tool names - ([ecfcda4](https://github.com/Aleph-Alpha/pharia-kernel/commit/ecfcda4eb812b94368f6d85a31a07f1c04f4cc05))

### Fixes

- Only load mcp servers in config with beta flag - ([c12e902](https://github.com/Aleph-Alpha/pharia-kernel/commit/c12e902964a2aa7ff3afde303c6c98079481c9dc))
- One bad mcp server allows to invoke tools on others - ([fe17a61](https://github.com/Aleph-Alpha/pharia-kernel/commit/fe17a61ae153ec4038b78740f6926ba40dc44870))

### Documentation

- Specify learnings about WIT - ([15119e4](https://github.com/Aleph-Alpha/pharia-kernel/commit/15119e403e3edd513b4880d79f855ed495711f34))

### Performance

- Do requests for list tool in parallel - ([6e654cf](https://github.com/Aleph-Alpha/pharia-kernel/commit/6e654cf5cb56cbb185761708e5fb68bb4ddeafd9))
- Tool actor answers request in parallel - ([fbcf053](https://github.com/Aleph-Alpha/pharia-kernel/commit/fbcf053adaec3f08beb83a035f6519f8dacea08d))

### Builds

- *(deps)* Bump the minor group across 1 directory with 36 updates - ([701e322](https://github.com/Aleph-Alpha/pharia-kernel/commit/701e3221d1223ad26aeb36dfba69ba9a086b9c09))


## [0.12.11](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.10...pharia-kernel-v0.12.11)

### Features

- Add complete v2 to csi shell - ([81d8816](https://github.com/Aleph-Alpha/pharia-kernel/commit/81d88164c59f8b7f1f7e92928a1b2f97f2e66970))
- Introduce complete-v2 in wit world with support for echo parameter - ([c1ef979](https://github.com/Aleph-Alpha/pharia-kernel/commit/c1ef9794574cb2f7c460d3448c31c09472915a56))

### Fixes

- Add since gate to tool interface - ([2cd0961](https://github.com/Aleph-Alpha/pharia-kernel/commit/2cd0961e63762a8ddab6f8188cdfdac352145471))

### Builds

- *(deps)* Update to aleph alpha client 0.27 - ([7313db7](https://github.com/Aleph-Alpha/pharia-kernel/commit/7313db728f0accfe2a8360f181a0b7d5bea60172))
- Remove unused tokio-test - ([cfecbbb](https://github.com/Aleph-Alpha/pharia-kernel/commit/cfecbbb90d3d5c69ce0183cd8a485ef505a066ad))


## [0.12.9](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.8...pharia-kernel-v0.12.9)

### Documentation

- Remove outdated references - ([fce9ab8](https://github.com/Aleph-Alpha/pharia-kernel/commit/fce9ab8d466a16dfb398b2edd251f285e2fb4cff))


## [0.12.8](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.7...pharia-kernel-v0.12.8)

### Documentation

- Update links for standalone deployment - ([2285c41](https://github.com/Aleph-Alpha/pharia-kernel/commit/2285c412efc06a2bb48d882a6eb15bcb771a9105))


## [0.12.7](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.6...pharia-kernel-v0.12.7)

### Features

- Write errors to stderr in case app init fails - ([5235b66](https://github.com/Aleph-Alpha/pharia-kernel/commit/5235b66a3398f37f57ee357c8c686c6b68022fab))


## [0.12.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.5...pharia-kernel-v0.12.6)

### Fixes

- Downgrade wasmtime to prevent rustix panic - ([4308274](https://github.com/Aleph-Alpha/pharia-kernel/commit/4308274941c0a9d59e12ca8783514338e3ad0d57))


## [0.12.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.4...pharia-kernel-v0.12.5)

### Features

- Expose otel sampling ratio in values.yaml - ([5ba0aa7](https://github.com/Aleph-Alpha/pharia-kernel/commit/5ba0aa79642c93395d94e467593d44acdbe13480))
- Pass tracestate to aleph alpha client - ([900617e](https://github.com/Aleph-Alpha/pharia-kernel/commit/900617eb7d06620c9473a7f85e4c0af5ec4d7d0b))
- Pass sampled flag to downstream services - ([e2f02c0](https://github.com/Aleph-Alpha/pharia-kernel/commit/e2f02c09cc8e412cbffa1d51e16f2ce812ad474c))
- Only include tracestate header if not empty - ([6c4cd07](https://github.com/Aleph-Alpha/pharia-kernel/commit/6c4cd074be045dba6c6a044835b33e8b44d1d7b8))
- Include tracestate in outgoing requests - ([a00e91d](https://github.com/Aleph-Alpha/pharia-kernel/commit/a00e91dcd2fcb8f01b73e4be527953c3062a8c3d))
- Better trace names - ([375e146](https://github.com/Aleph-Alpha/pharia-kernel/commit/375e14645359b73b74051bfd9c804fc05f306ebe))
- Do not include original error in auth error message - ([161cf9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/161cf9a6a45f4d6d17b26f4130eb7ff91134d381))
- Skill loader gets tracing context - ([58d0959](https://github.com/Aleph-Alpha/pharia-kernel/commit/58d0959682d43b63a677f56c59304bb0f5e8e68c))
- New tracing span for skill loading - ([67f27c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/67f27c8e79185b1ec0fbeecea4705d03247a928b))
- Expose better error message for auth errors - ([55cf006](https://github.com/Aleph-Alpha/pharia-kernel/commit/55cf006e9fec443f9a0004532da0f66059853bab))
- Create span for auth calls - ([dc5ab4d](https://github.com/Aleph-Alpha/pharia-kernel/commit/dc5ab4d201d860388b2c339b57a5a2dac3cea669))
- Forward traceparent to iam service - ([bb655d7](https://github.com/Aleph-Alpha/pharia-kernel/commit/bb655d79234878d5aa99b74212901035e97388fa))
- Nest concurrent requests into span - ([9c00b81](https://github.com/Aleph-Alpha/pharia-kernel/commit/9c00b81f6e0cf4a291bb02b1c26b8801412b9acd))
- Start a new span for run_function - ([98474fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/98474fddc30ea23ac936e071ee6337653efe41aa))
- Update span target to "pharia-kernel::csi" - ([c599f6e](https://github.com/Aleph-Alpha/pharia-kernel/commit/c599f6ef2612fd4e61c70931bceb46fa1f0c1c09))
- Start span for skill execution - ([461eb8e](https://github.com/Aleph-Alpha/pharia-kernel/commit/461eb8e5380fd2093279192a409de387e3c86180))
- Traceheader is forwarded to document index - ([04c702c](https://github.com/Aleph-Alpha/pharia-kernel/commit/04c702c973ff7e4a4c94a484de9ea45af1801960))
- Pass trace context to inference client - ([c66e9af](https://github.com/Aleph-Alpha/pharia-kernel/commit/c66e9af321d380ad9f1e4ea7ea4cc79ebdf0c30a))
- Create child span for chat request - ([296c839](https://github.com/Aleph-Alpha/pharia-kernel/commit/296c8395240769cb1ed31b4b10e3cc9ee2f0d70d))
- Context event emitted for chat request - ([39b6bc4](https://github.com/Aleph-Alpha/pharia-kernel/commit/39b6bc4ddb74364a473855180d62c188ef0c90cb))
- Enable file-based caching for wasmtime to reduce cold starts even - ([6637093](https://github.com/Aleph-Alpha/pharia-kernel/commit/6637093bc99d65023f7675d4b522f43c97066855))
- Add trace layer to see response start and end events - ([28e0fa8](https://github.com/Aleph-Alpha/pharia-kernel/commit/28e0fa87d0f4b4cb3fdb32bf448fe0ea6deae113))
- Otel sampling ratio is taken from env - ([4727fea](https://github.com/Aleph-Alpha/pharia-kernel/commit/4727fea6722845351a68e1802cc2ab7c63a742dc))

### Fixes

- Situate language conversion in tracing context - ([e0561e1](https://github.com/Aleph-Alpha/pharia-kernel/commit/e0561e179f8798207ab3110382337c969600f6d2))
- Search actor logs to trace context - ([0c99656](https://github.com/Aleph-Alpha/pharia-kernel/commit/0c9965682e7b8bf297cce5f9ff54421e4f191466))
- Remove tracing from http client - ([fc578b5](https://github.com/Aleph-Alpha/pharia-kernel/commit/fc578b51fcc0b0966eb44bf3cc71b3d00144c159))
- Only drop tracing context on stream end - ([99ca7a0](https://github.com/Aleph-Alpha/pharia-kernel/commit/99ca7a0c469929322f6fd1c23f41336ff3325577))
- Increment csi request metrics by total requests - ([0f20aba](https://github.com/Aleph-Alpha/pharia-kernel/commit/0f20aba3e3c26922cbc33eacd3dc999b9a2ba450))
- Pass correct child context to inference requests - ([0fa118c](https://github.com/Aleph-Alpha/pharia-kernel/commit/0fa118cbeefea10b1c766ff13fbdf78ad7aba785))
- Do not unwrap if span id is not set - ([86d260c](https://github.com/Aleph-Alpha/pharia-kernel/commit/86d260cbc15496a8a80ec17a38f0599137a68ea8))
- Set log level to info on traces - ([b90ce37](https://github.com/Aleph-Alpha/pharia-kernel/commit/b90ce372f079d06354a63301e4e21fa64067cf8e))
- Define axum otel middleware outside of service builder - ([0edccf4](https://github.com/Aleph-Alpha/pharia-kernel/commit/0edccf4e5fdb59d02e5f22d695feab84bed04d6f))

### Documentation

- Remove unneeded spaces before shell cmds - ([e5c0344](https://github.com/Aleph-Alpha/pharia-kernel/commit/e5c034475e0a89f18fa4b3fdd46333f3b6b704f1))
- Remove detach option when starting jaeger - ([94b9843](https://github.com/Aleph-Alpha/pharia-kernel/commit/94b9843a02175a63cb2c06ecb49704d0d200d7ee))
- Specify why we need to do tracing differently - ([c7e1331](https://github.com/Aleph-Alpha/pharia-kernel/commit/c7e1331bfa3ac211f51adc8c36a805d0b073410d))
- Specify purpose of context_event macro - ([4f74a0f](https://github.com/Aleph-Alpha/pharia-kernel/commit/4f74a0f1fadb498eafdb63e50fd0593ce35cf602))
- Specify why we use tracing_level_info feature for axum middleware - ([c44c9ff](https://github.com/Aleph-Alpha/pharia-kernel/commit/c44c9ff48953eaa3f644fda47a8fbc1a44c6a1bb))
- Explain servicebuilder nesting - ([a3f4cd3](https://github.com/Aleph-Alpha/pharia-kernel/commit/a3f4cd3195d2f982dfe3fd0465b0dbee3083d6d1))

### Builds

- *(deps)* Bump webpki-roots from 0.26.9 to 0.26.11 - ([e1f9f61](https://github.com/Aleph-Alpha/pharia-kernel/commit/e1f9f611f9f419d1d4fa00364e39cff86b8468e7))
- *(deps)* Bump the minor group with 9 updates - ([ec40ee9](https://github.com/Aleph-Alpha/pharia-kernel/commit/ec40ee913b8cf4ad3cfcb7490c99ccee30621b1d))
- *(deps)* Remove unused dependencies found by cargo shear - ([21c23b1](https://github.com/Aleph-Alpha/pharia-kernel/commit/21c23b1cb33fd751ce37ce940b44cfa470feda06))
- *(deps)* Bump the minor group with 20 updates - ([692d39e](https://github.com/Aleph-Alpha/pharia-kernel/commit/692d39e0aca3c9700d4c048eac7dad1b75a6ff4e))
- *(deps)* Bump the minor group across 1 directory with 27 updates - ([dc95088](https://github.com/Aleph-Alpha/pharia-kernel/commit/dc95088c7c854e3ba5d68a95dde3b2933d43b965))
- *(deps)* Update aleph alpha client to 0.24 - ([4dba514](https://github.com/Aleph-Alpha/pharia-kernel/commit/4dba514c215c567fa40ea5167668bee3f7976bd3))
- *(deps)* Bump the minor group with 4 updates - ([3e3059c](https://github.com/Aleph-Alpha/pharia-kernel/commit/3e3059c3229a4b361edeb7f449a25cd09749ed05))
- *(deps)* Bump the minor group with 8 updates - ([2a08c40](https://github.com/Aleph-Alpha/pharia-kernel/commit/2a08c40fc174965fd5b5b4a08165803531a90b60))


## [0.12.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.3...pharia-kernel-v0.12.4)

### Features

- Support filtering for skill type when listing skills - ([a81477b](https://github.com/Aleph-Alpha/pharia-kernel/commit/a81477b4a0f1f900d161a25f17cfc6c2728fb669))
- Add native chat skill - ([587a290](https://github.com/Aleph-Alpha/pharia-kernel/commit/587a2908c81b51b19e64d3f64ed91a1056946e4a))

### Builds

- *(deps)* Bump winnow from 0.7.6 to 0.7.7 in the minor group - ([7e49248](https://github.com/Aleph-Alpha/pharia-kernel/commit/7e49248755ef5a2e3acdea8eedbd8b113139a9d1))
- *(deps)* Bump astral-sh/setup-uv from 5 to 6 - ([8922f7a](https://github.com/Aleph-Alpha/pharia-kernel/commit/8922f7aa4eb780cdcbc0678a904c2a17b0af0013))


## [0.12.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.1...pharia-kernel-v0.12.2)

### Features

- Add wasmtime file-based cache support if enabled from config - ([6d4489f](https://github.com/Aleph-Alpha/pharia-kernel/commit/6d4489f17c2b07d3a41d1c9c072be9ee3d02ff79))


## [0.12.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.12.0...pharia-kernel-v0.12.1)

### Features

- Fit more skills in cache with new measurements - ([235d274](https://github.com/Aleph-Alpha/pharia-kernel/commit/235d274c43be4a34ee736560bbbabed776bd256d))


## [0.12.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.11.20...pharia-kernel-v0.12.0)

### Features

- Add wasmtime cache configuration options - ([43e319a](https://github.com/Aleph-Alpha/pharia-kernel/commit/43e319a61f88ac57c86e12f1fb0e4072d9cbd057))

### Builds

- *(deps)* Bump the minor group with 6 updates - ([40338bf](https://github.com/Aleph-Alpha/pharia-kernel/commit/40338bfad79fed7e23361e71205502a15417c22b))
- *(deps)* Bump brotli-decompressor from 4.0.2 to 4.0.3 - ([2cf79a7](https://github.com/Aleph-Alpha/pharia-kernel/commit/2cf79a760c664de31ffdea56b79b3c9775242dc2))
- *(deps)* Bump the minor group with 9 updates - ([217dd72](https://github.com/Aleph-Alpha/pharia-kernel/commit/217dd722ebb4404ce1d34975d3c226a9f584e971))
- *(deps)* Bump the minor group with 3 updates - ([b8798e1](https://github.com/Aleph-Alpha/pharia-kernel/commit/b8798e142eced4d662bde0def0f403d97715633d))


## [0.11.20](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.11.19...pharia-kernel-v0.11.20)

### Features

- Be more verbose than logging a request error connecting to an OCI registry - ([c9bff4e](https://github.com/Aleph-Alpha/pharia-kernel/commit/c9bff4ecb6c36471d8872d8e40e3dcf978f36203))
- Specify service name instead of reading from env - ([cda8372](https://github.com/Aleph-Alpha/pharia-kernel/commit/cda8372d191c2094ab155d772059e42246916500))

### Fixes

- Do not add instrument macro on index endpoint - ([ea66fdf](https://github.com/Aleph-Alpha/pharia-kernel/commit/ea66fdf14c1cd5917c5b48e2b7b04576b2ed36df))
- Do not set sampling to always on - ([4c81530](https://github.com/Aleph-Alpha/pharia-kernel/commit/4c815300a22e76f3af0826785253968770f28b7b))
- Explicitly set service name for ressource - ([ca9487a](https://github.com/Aleph-Alpha/pharia-kernel/commit/ca9487adbc7e2c6b413f3b838ea171f56c03a0db))

### Documentation

- Fix arrow direction in block diagram - ([0a9c923](https://github.com/Aleph-Alpha/pharia-kernel/commit/0a9c923fffe5e55559c333f84190fa19c3d5056a))
- Specify that otel scope name is set when creating a tracer - ([d23f958](https://github.com/Aleph-Alpha/pharia-kernel/commit/d23f958867566f10f8feb9b9c279d9884ca66982))

### Builds

- *(deps)* Bump the minor group with 3 updates - ([0e523de](https://github.com/Aleph-Alpha/pharia-kernel/commit/0e523dea1beb177b40710783c3b46e611104347c))
- *(deps)* Bump libc from 0.2.171 to 0.2.172 in the minor group - ([c77f531](https://github.com/Aleph-Alpha/pharia-kernel/commit/c77f5313be2b3c04d4bae7d5deb405d573a4e96a))
- *(deps)* Bump the minor group with 5 updates - ([054fd7a](https://github.com/Aleph-Alpha/pharia-kernel/commit/054fd7a3b48c4a907417d587c0a238594dce7491))
- *(deps)* Axum tracing is a dev-dependency only - ([29be6fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/29be6fd82f87a4885fa5bf89263b4aebd12f750b))


## [0.11.19](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.11.18...pharia-kernel-v0.11.19)

### Builds

- *(deps)* Bump the minor group across 1 directory with 10 updates - ([b8e5069](https://github.com/Aleph-Alpha/pharia-kernel/commit/b8e50692add48ea69934f9ab1e9bf5a8f28cba6d))


## [0.11.18](https://github.com/Aleph-Alpha/pharia-kernel/compare/pharia-kernel-v0.11.17...pharia-kernel-v0.11.18)

### Features

- Add incremental compilation cache for WebAssembly components - ([80b5a00](https://github.com/Aleph-Alpha/pharia-kernel/commit/80b5a0051d8eb0f597603ad22addcc8f5f3e873a))

### Fixes

- Do not drop otel guard to prevent early shutdown - ([c1fcbc5](https://github.com/Aleph-Alpha/pharia-kernel/commit/c1fcbc5d54167a40d4d3781150fea9ff384465ff))


## [0.11.17](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.16...v0.11.17)

### Features

- Add metrics for skill fetch duration - ([baecd44](https://github.com/Aleph-Alpha/pharia-kernel/commit/baecd441765365b23ddccd0dde942fb1cabbb3e7))


## [0.11.16](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.15...v0.11.16)

### Documentation

- Add block diagram Kernel in Pharia AI - ([a273a10](https://github.com/Aleph-Alpha/pharia-kernel/commit/a273a10d68006f5d9403b7959b3e843d6742448b))
- Updated Block Diagram - ([8b28875](https://github.com/Aleph-Alpha/pharia-kernel/commit/8b2887540fc62a25dcc78b3fec68c455088bfdf4))

### Builds

- *(deps)* Bump the minor group with 10 updates - ([fb309b2](https://github.com/Aleph-Alpha/pharia-kernel/commit/fb309b2cc326e9498b4e8d387edc46c1cecb2c91))


## [0.11.15](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.14...v0.11.15)

### Fixes

- Handle request stops listening before completion - ([5345f80](https://github.com/Aleph-Alpha/pharia-kernel/commit/5345f80b22af8ac6d535844975d44d753c12bf0f))


## [0.11.14](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.13...v0.11.14)

### Features

- Automatically determine target cache size - ([c5946a5](https://github.com/Aleph-Alpha/pharia-kernel/commit/c5946a5ab1343cf7595c20259178c569b43f42fc))

### Documentation

- Rename `example` namespace in examples - ([536623b](https://github.com/Aleph-Alpha/pharia-kernel/commit/536623b240ce4169bf988a2ae183f330902ab327))

### Builds

- *(deps)* Bump miniz_oxide from 0.8.5 to 0.8.7 in the minor group - ([1e24d62](https://github.com/Aleph-Alpha/pharia-kernel/commit/1e24d6230b4a5dc741a469cf2ddbb3a30bb67120))


## [0.11.13](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.12...v0.11.13)

### Features

- Read memory request and limit configuration options from env - ([bb4ace7](https://github.com/Aleph-Alpha/pharia-kernel/commit/bb4ace7e7c3b06b9cbf846d2c32b21453f17c74a))

### Documentation

- Add examples for message stream response - ([7892cc4](https://github.com/Aleph-Alpha/pharia-kernel/commit/7892cc413bcf12abfd39d4486f3a070757d00e03))

### Builds

- *(deps)* Bump the minor group with 2 updates - ([bd58a7b](https://github.com/Aleph-Alpha/pharia-kernel/commit/bd58a7bbb34fdf4b034593a95082d071558bd52d))


## [0.11.12](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.11...v0.11.12)

### Features

- Add skill cache metrics tracking - ([d7c56f9](https://github.com/Aleph-Alpha/pharia-kernel/commit/d7c56f99e7073549e35cb1dc960cb401227cfb33))
- Improve memory performance for cached skills - ([c4c0f41](https://github.com/Aleph-Alpha/pharia-kernel/commit/c4c0f4102a49bdf09e6bfc449da08fe457ac5f23))

### Builds

- *(deps)* Bump the minor group with 4 updates - ([95f2498](https://github.com/Aleph-Alpha/pharia-kernel/commit/95f2498c207dc7b582a55c862bfed324d4179a09))


## [0.11.11](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.10...v0.11.11)

### Features

- Better error message for SkillLoadError - ([c25155c](https://github.com/Aleph-Alpha/pharia-kernel/commit/c25155cc285b6e15a83139b3aaadb48dab5e6672))

### Fixes

- Simplify controlflow consuming skill events. Resolving an issue which could caused errors to be inserted twice into the execution stream - ([40cc263](https://github.com/Aleph-Alpha/pharia-kernel/commit/40cc2639a316caad812b7350be7ea2e839b18b83))

### Builds

- _(deps)_ Bump the minor group with 3 updates - ([b382143](https://github.com/Aleph-Alpha/pharia-kernel/commit/b3821432507472feadf47b4718d359a4835d51cb))

## [0.11.10](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.9...v0.11.10)

### Features

- Insert errors into streams on invalid message state transitions - ([a3e1196](https://github.com/Aleph-Alpha/pharia-kernel/commit/a3e11961d99d26003bb3713c40d14b228d34e1d0))

### Fixes

- Skills emitting invalid Json in stream are now correctly logged as being buggy. - ([691911c](https://github.com/Aleph-Alpha/pharia-kernel/commit/691911c8f02daf8211e2d6b962a861b1af91acc2))

### Documentation

- Fix spelling of operator - ([247cb6b](https://github.com/Aleph-Alpha/pharia-kernel/commit/247cb6bc9a9227e364c52c83eeee12b9976fd6a6))
- Fix typos in skill event doc string - ([014a515](https://github.com/Aleph-Alpha/pharia-kernel/commit/014a51569d3bf2f0924443cf3ac6f1ff7e4fc851))
- Fix multiple small typos - ([5cad009](https://github.com/Aleph-Alpha/pharia-kernel/commit/5cad0096d7d4baa1a8db608c5602d7027001ab27))

### Builds

- _(deps)_ Bump the minor group with 8 updates - ([28aee9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/28aee9ad3281332698ddfe70b4326ee8832a4fd4))

## [0.11.9](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.8...v0.11.9)

### Fixes

- Use Default::default() for FeatureSet instead of hardcoded values - ([c053b03](https://github.com/Aleph-Alpha/pharia-kernel/commit/c053b03a2eff23c58cf4e6021073f9bc83d29fd2))

## [0.11.8](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.7...v0.11.8)

### Features

- Promote /message-stream endpoint to stable - ([edf5354](https://github.com/Aleph-Alpha/pharia-kernel/commit/edf53544e7c29436ddd30a257e8d2edc15217b59))
- Promote streaming features from unstable to stable - ([edcb5fa](https://github.com/Aleph-Alpha/pharia-kernel/commit/edcb5fa42226c542ba3aa00f49584dec2363570d))

### Builds

- _(deps)_ Bump the minor group with 3 updates - ([677ad11](https://github.com/Aleph-Alpha/pharia-kernel/commit/677ad114eb6275dfc86faa3f02818c8271f0bed5))

## [0.11.7](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.6...v0.11.7)

### Features

- Invalid user input logs as info - ([fe4cea1](https://github.com/Aleph-Alpha/pharia-kernel/commit/fe4cea1f5c22dfa43a8d95d8b6aa01d865d645e1))
- Introduce minimal tracing for skill invocation errors - ([14c2b2d](https://github.com/Aleph-Alpha/pharia-kernel/commit/14c2b2da87da72bca73f954ae0f166c2df721c19))

### Fixes

- Return correct error message when stream skill is invoked via run - ([f6b7b9c](https://github.com/Aleph-Alpha/pharia-kernel/commit/f6b7b9cd925edb1541e584a23745f98a1314a95e))
- TOO_MANY_REQUEST is treated as recoverable for configuration - ([1ec52df](https://github.com/Aleph-Alpha/pharia-kernel/commit/1ec52dfeb2c0ede099c6907bad3adfd1d3214683))
- Syntax errors in namespace configuration are now classified as unrecoverable - ([4bddd8a](https://github.com/Aleph-Alpha/pharia-kernel/commit/4bddd8a257508b7a58f5d319638ab9fea9813612))

### Documentation

- Update URL to Python SDK documentation - ([ff20b43](https://github.com/Aleph-Alpha/pharia-kernel/commit/ff20b43150f17240ec79988a605a3daaeecc0f0e))
- Fix typo - ([6d23b76](https://github.com/Aleph-Alpha/pharia-kernel/commit/6d23b768efee3231c05bb00803b4f328add8bd58))

### Builds

- _(deps)_ Bump the minor group with 5 updates - ([16e5695](https://github.com/Aleph-Alpha/pharia-kernel/commit/16e56952decc44cd6018b3657e89917d4ad8ec19))
- _(deps)_ Bump the minor group with 9 updates - ([5fe763f](https://github.com/Aleph-Alpha/pharia-kernel/commit/5fe763fb64432389fbf7bb24ae30c926caf6e6e6))

## [0.11.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.5...v0.11.6)

### Fixes

- Metrics now only measure message execution time - ([a904ba6](https://github.com/Aleph-Alpha/pharia-kernel/commit/a904ba65609e04d302879b650b1b3923b2caa4ef))
- Function metrics only measure execution time without fetching - ([a03e4ab](https://github.com/Aleph-Alpha/pharia-kernel/commit/a03e4abf7e588e16cfbe2c3b87c09ae1498ff59a))

### Documentation

- Fix typo - ([9deff2f](https://github.com/Aleph-Alpha/pharia-kernel/commit/9deff2f64da780ed926312b319115d4aa5ca17cc))

### Builds

- _(deps)_ Bump the minor group with 10 updates - ([875b4d1](https://github.com/Aleph-Alpha/pharia-kernel/commit/875b4d1567031dd26e52c13b7938a733d56834cf))

## [0.11.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.4...v0.11.5)

### Fixes

- Update CSI server-side event names - ([c899146](https://github.com/Aleph-Alpha/pharia-kernel/commit/c89914606ab74c0618e4b420c8694c341baf0263))

## [0.11.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.3...v0.11.4)

### Documentation

- Typo - ([434fde8](https://github.com/Aleph-Alpha/pharia-kernel/commit/434fde81fed117fe1393d5aea15ef75519d49ce7))

## [0.11.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.2...v0.11.3)

### Fixes

- Correct input type for chunk_with_offsets route - ([a1b6149](https://github.com/Aleph-Alpha/pharia-kernel/commit/a1b6149178700b9fc30e7876565b23a626a6a472))

## [0.11.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.1...v0.11.2)

### Features

- Enable unstable streaming skill module execution - ([bb45207](https://github.com/Aleph-Alpha/pharia-kernel/commit/bb4520764f518e7a1dd41a276d1fa3301e474043))
- Rename Message start -> Message begin - ([e17b499](https://github.com/Aleph-Alpha/pharia-kernel/commit/e17b499f8f9c80f8c8ff5fa62237413550484e09))
- Messages emit start and end event - ([443101d](https://github.com/Aleph-Alpha/pharia-kernel/commit/443101d0eda48af23a7bfa99951cbb2291342420))
- Correctly report if functions are erroneously invoked as generators. - ([ee68694](https://github.com/Aleph-Alpha/pharia-kernel/commit/ee6869406d9d810b9e0314ab10159d5becbda751))

### Fixes

- Event stream for tell me a joke skill - ([5fc9286](https://github.com/Aleph-Alpha/pharia-kernel/commit/5fc928669cbc4a6be218ead7e1ec8bb40c785698))

### Builds

- _(deps)_ Bump the wasmtime group with 17 updates - ([3b14786](https://github.com/Aleph-Alpha/pharia-kernel/commit/3b14786a7f3513179b36a4324653158ad1f141ef))
- _(deps)_ Bump the minor group with 83 updates - ([027e75b](https://github.com/Aleph-Alpha/pharia-kernel/commit/027e75bd4da90b17942d155aa9e41d785e6141ab))
- _(deps)_ Bump the minor group with 2 updates - ([04f3fb0](https://github.com/Aleph-Alpha/pharia-kernel/commit/04f3fb084485358b1e809f86265eb32733514f50))

## [0.11.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.11.0...v0.11.1)

### Features

- Rename "stream" route from "chat" - ([ccb86fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/ccb86fd2b115acd686c720f1e7f8ecb9a646bd95))

## [0.11.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.10.2...v0.11.0)

### Features

- Add chat streaming endpoint to CSI shell - ([e7e0c94](https://github.com/Aleph-Alpha/pharia-kernel/commit/e7e0c941462efe84a3184d7a93fb16280c8d0cd4))
- Connect unstable chat streaming into skills - ([ef294ea](https://github.com/Aleph-Alpha/pharia-kernel/commit/ef294ea20eb590e7e72eb59af843ad3e8ffef9be))
- Add completion stream to dev csi - ([e70e654](https://github.com/Aleph-Alpha/pharia-kernel/commit/e70e654c51ab4fe4dd97ffc2f87679f6717a6534))
- Hook up unstable completion streaming - ([5308977](https://github.com/Aleph-Alpha/pharia-kernel/commit/53089777565babd4203442a47e47e570ba72047a))
- Convert AppConfig to builder lite pattern - ([ef9b72d](https://github.com/Aleph-Alpha/pharia-kernel/commit/ef9b72df65611467c6b11362d4c0cfeac73d67bf))
- Add skill_type to metadata - ([347c92e](https://github.com/Aleph-Alpha/pharia-kernel/commit/347c92ec9421f433e48bd86e3ea96533b4323683))

### Builds

- _(deps)_ Bump the minor group with 5 updates - ([2e0ebc8](https://github.com/Aleph-Alpha/pharia-kernel/commit/2e0ebc81b8907f2329ad4dc00024d8b2bc0e48d2))
- _(deps)_ Bump rustls from 0.23.24 to 0.23.25 in the minor group - ([0e2ad64](https://github.com/Aleph-Alpha/pharia-kernel/commit/0e2ad64a6b2776a09384c554238ab3611d87cd13))
- _(deps)_ Bump the minor group across 1 directory with 16 updates - ([3522689](https://github.com/Aleph-Alpha/pharia-kernel/commit/352268918c28fbf9025858744a98be72c816a5de))

## [0.10.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.10.1...v0.10.2)

### Features

- Return text instead of Json Errors - ([4af08ad](https://github.com/Aleph-Alpha/pharia-kernel/commit/4af08ade838355facf00c9b826953e0826443005))
- Hide metadata open-api doc in production - ([81c08e1](https://github.com/Aleph-Alpha/pharia-kernel/commit/81c08e1cc424dc2f6975d4097882a6f00a507375))
- Only allow "test-beta" namespace to access hardcoded skills - ([e026d51](https://github.com/Aleph-Alpha/pharia-kernel/commit/e026d517afe870d0b2d660c03ca3147417080c08))
- Better error messages - ([cfaf812](https://github.com/Aleph-Alpha/pharia-kernel/commit/cfaf812a5784aa4db3a6736219f830a9f1d77bfb))
- Label logic and runtime errors in metrics - ([c016dfa](https://github.com/Aleph-Alpha/pharia-kernel/commit/c016dfae481c2c4f9236953208d2951a54c15665))

### Builds

- _(deps)_ Bump the minor group across 1 directory with 11 updates - ([be6d7f0](https://github.com/Aleph-Alpha/pharia-kernel/commit/be6d7f0bc604279aa697c95041b994a12eba9057))

## [0.10.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.10.0...v0.10.1)

### Features

- Handling of runtime errors for chat skills - ([54babdf](https://github.com/Aleph-Alpha/pharia-kernel/commit/54babdf781c9c34ee647712b2c9034108dced541))
- Do not send useless spans - ([fc86dfc](https://github.com/Aleph-Alpha/pharia-kernel/commit/fc86dfce565575f7af4368dc2834c5aff5f51864))

### Documentation

- Tam for how inference is invoked from a skill - ([b9201de](https://github.com/Aleph-Alpha/pharia-kernel/commit/b9201de9d627fad301d6515ffa730583e7ad1afb))

## [0.10.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.9...v0.10.0)

### Features

- Add metrics for chat runtime metrics - ([458d5ae](https://github.com/Aleph-Alpha/pharia-kernel/commit/458d5aec90883ce2ccd1f2cb53edc0bfa65a7aa2))
- Send message delta in JSON format - ([1c22f35](https://github.com/Aleph-Alpha/pharia-kernel/commit/1c22f35b6c0300153acb33dbe743183962d98a6b))
- Handle saboteur chat skill - ([1a33eb3](https://github.com/Aleph-Alpha/pharia-kernel/commit/1a33eb3be7d94c9e6fa0995f5f24bbc9577fe1ae))
- Handle chat skill not found - ([693919b](https://github.com/Aleph-Alpha/pharia-kernel/commit/693919b694d702da6c3ac3ab4fbdecf556073a2c))
- Handle string values only for feature set - ([d490787](https://github.com/Aleph-Alpha/pharia-kernel/commit/d4907876ab73b26303d72c979641f5ee8dfbd869))
- Separate OpenAPI docs for beta - ([cdb1016](https://github.com/Aleph-Alpha/pharia-kernel/commit/cdb10167a1bbba5db80a7640304b261a867a29a5))
- Init chat skill endpoint - ([cd98950](https://github.com/Aleph-Alpha/pharia-kernel/commit/cd989503ffa4bc8547dcf20fbd5bfc464c1f8031))
- Add skills to /v1/ api - ([db59b9b](https://github.com/Aleph-Alpha/pharia-kernel/commit/db59b9b847ad9d26a840d29c59b0c42855677a9d))

### Fixes

- Handle invalid config access token - ([4513bdd](https://github.com/Aleph-Alpha/pharia-kernel/commit/4513bdd1aa503f8b49c84816f7305b8f4acb0c01))

### Documentation

- Remove invalid request response - ([fff801e](https://github.com/Aleph-Alpha/pharia-kernel/commit/fff801ee263e080d7c7d6515b882bf77100c8e18))

### Builds

- _(deps)_ Bump the minor group with 3 updates - ([b7c721f](https://github.com/Aleph-Alpha/pharia-kernel/commit/b7c721f58ddf9613c1c98a51c7e2addfa8ef2d5f))
- _(deps)_ Bump the minor group with 11 updates - ([2ee4d97](https://github.com/Aleph-Alpha/pharia-kernel/commit/2ee4d97abf628f175b0dac46bf7c05733f75daef))
- _(deps)_ Bump the minor group with 14 updates - ([7aa855e](https://github.com/Aleph-Alpha/pharia-kernel/commit/7aa855eca0a1ab6919a8b17fb72681acd2f9e94a))
- _(deps)_ Bump the minor group with 24 updates - ([4c640e7](https://github.com/Aleph-Alpha/pharia-kernel/commit/4c640e74ecf843ad984b71fde6314e7911abd0c8))
- _(deps)_ Bump the minor group with 48 updates - ([93da08b](https://github.com/Aleph-Alpha/pharia-kernel/commit/93da08b0ed19e2d058cb49f07487a8406524f322))
- Standardize operation ID in OpenAPI - ([22b1fd7](https://github.com/Aleph-Alpha/pharia-kernel/commit/22b1fd7b3529927593ecc3a2cff2bc8ef794bec4))

## [0.9.9](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.8...v0.9.9)

### Features

- Stabilize chunk with offsets in 0.3 wit world - ([3e9ba6e](https://github.com/Aleph-Alpha/pharia-kernel/commit/3e9ba6e92b71811ea5a2685a6f6d311427efe05f))

## [0.9.7](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.6...v0.9.7)

### Features

- Update text splitter to 0.24.1 and support char offset - ([5aded3c](https://github.com/Aleph-Alpha/pharia-kernel/commit/5aded3c0a85cd883aedcd5bb7d13b258c84ad311))
- Add chunk with offsets support - ([1e99b34](https://github.com/Aleph-Alpha/pharia-kernel/commit/1e99b3455d69352b923838b4b48ce67a7d70f138))
- Add unstable WIT interfaces for chunking with offsets - ([c7f2bd0](https://github.com/Aleph-Alpha/pharia-kernel/commit/c7f2bd05145757e0fb4d08ceb40e6a727dcbcbab))

### Documentation

- Update Pharia Kernel to PhariaKernel - ([0798857](https://github.com/Aleph-Alpha/pharia-kernel/commit/079885736e5fbaafdb7a98067bbd95b20996e01f))
- Streamline names of endpoints and skill case - ([7c7deeb](https://github.com/Aleph-Alpha/pharia-kernel/commit/7c7deebd5f273e77e5365acad89e011642fd7a3a))
- Add skill-metadata endpoint to openapi description - ([ce30ba8](https://github.com/Aleph-Alpha/pharia-kernel/commit/ce30ba8c1fe535903ddcbdcded33e50542d33778))

### Builds

- _(deps)_ Bump the minor group with 40 updates - ([d36df6d](https://github.com/Aleph-Alpha/pharia-kernel/commit/d36df6d512d25c38527b85ee5c0c316f374cf00d))

## [0.9.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.5...v0.9.6)

### Features

- Stabilize explain in wit world - ([c79d007](https://github.com/Aleph-Alpha/pharia-kernel/commit/c79d00726e227379d54b513aa78281c67fcc48b0))

### Builds

- _(deps)_ Bump the wasmtime group with 16 updates - ([50cd082](https://github.com/Aleph-Alpha/pharia-kernel/commit/50cd082531f08f6204e45fa822fc467cda26337a))
- _(deps)_ Bump the minor group with 8 updates - ([3c166b5](https://github.com/Aleph-Alpha/pharia-kernel/commit/3c166b5b3caa6a3bd8dcb9d65575a8212268f98c))
- _(deps)_ Bump the minor group with 7 updates - ([fa8c199](https://github.com/Aleph-Alpha/pharia-kernel/commit/fa8c19903d9d3392a3e504554f564486135863e8))

## [0.9.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.4...v0.9.5)

### Features

- Put explain behind unstable flag in wit world - ([19f0521](https://github.com/Aleph-Alpha/pharia-kernel/commit/19f052143fa15a5b60e64fb0766c694adf752703))
- Expose explanation via csi shell - ([f84c2dd](https://github.com/Aleph-Alpha/pharia-kernel/commit/f84c2dd8c78764a5054486ae57c9c58a62e545cd))
- Expose explain in wit world - ([04237ff](https://github.com/Aleph-Alpha/pharia-kernel/commit/04237ffa7c612c0588523a6365afc4a8380a0593))

### Fixes

- Use try from to parse inference explain response - ([b709d13](https://github.com/Aleph-Alpha/pharia-kernel/commit/b709d135449d49143b11558b2ffdbfd1e640ecc6))

## [0.9.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.3...v0.9.4)

### Fixes

- Remove unneeded modality filter exposed in csi shell search - ([c0f326e](https://github.com/Aleph-Alpha/pharia-kernel/commit/c0f326ee449e538a805ffa44f6d3e09c2373449e))

### Documentation

- Specify clean up after wit world change in readme - ([04e01a3](https://github.com/Aleph-Alpha/pharia-kernel/commit/04e01a3ede311a0aaeddabc5a7a70bb699ef7186))
- Specify directory option for namespace - ([ad89c92](https://github.com/Aleph-Alpha/pharia-kernel/commit/ad89c92028d3cc90a758e5da5859e383c12cf811))
- Add explaining comments to document index wit interface - ([19ae8d3](https://github.com/Aleph-Alpha/pharia-kernel/commit/19ae8d3397130c8234e9278ceb8531ac8e3681fa))

### Builds

- _(deps)_ Bump the minor group with 3 updates - ([578238d](https://github.com/Aleph-Alpha/pharia-kernel/commit/578238d84cee8aa85b861837ed8c7d89e85102a0))
- _(deps)_ Bump the minor group with 14 updates - ([4d04a75](https://github.com/Aleph-Alpha/pharia-kernel/commit/4d04a758c62e40fbced8b23a451922ddb9e14886))
- _(deps)_ Bump fake from 3.1.0 to 4.0.0 - ([2a6227e](https://github.com/Aleph-Alpha/pharia-kernel/commit/2a6227ec2d4d1c7920fdd97847eba12c9ab1159e))
- _(deps)_ Bump aleph-alpha-client in the minor group - ([326ad40](https://github.com/Aleph-Alpha/pharia-kernel/commit/326ad4060c80659dd2c78aed0014e1ce6cde24c4))

## [0.9.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.9.2...v0.9.3)

### Fixes

- Remove wrongly added vec around serialization of csi chunking request - ([751308b](https://github.com/Aleph-Alpha/pharia-kernel/commit/751308b109ec98ab63fb1680acd9d951095f4108))

### Documentation

- Fix listing - ([d21f6dd](https://github.com/Aleph-Alpha/pharia-kernel/commit/d21f6dd205ad28084af974f0cbd6cf12393c76d4))
- Describe release flow - ([4a90bf6](https://github.com/Aleph-Alpha/pharia-kernel/commit/4a90bf61186846a6c76f77679fa467f45b1ad791))

## [0.9.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.8.0...v0.9.0)

### Features

- Finalize v0.3 WIT release - ([5987a72](https://github.com/Aleph-Alpha/pharia-kernel/commit/5987a727e2235809fccedd5c1ddbce056e13b52b))
- [**breaking**] Remove deprecated execute_skill endpoint - ([71e4be5](https://github.com/Aleph-Alpha/pharia-kernel/commit/71e4be599f70e8d42a2626ac6712c31fe1d39637))
- Expose search filters via CSI shell - ([963f482](https://github.com/Aleph-Alpha/pharia-kernel/commit/963f482438dac8d6f9149b0ed4185cb91a23ff3b))
- Accept multiple filters for search request - ([e0de977](https://github.com/Aleph-Alpha/pharia-kernel/commit/e0de977aeb648ae5e1f2eda38ce92f59e8c2cbda))
- Support search with metadata filter - ([c7732f6](https://github.com/Aleph-Alpha/pharia-kernel/commit/c7732f682772791623ff1f9bf2043c4021b1fe02))

### Fixes

- Expose modality in csi shell - ([e6ff26c](https://github.com/Aleph-Alpha/pharia-kernel/commit/e6ff26c76a92e7d04fe697080a2a1201e33488a0))
- Update since tag for language enum in WIT world - ([da640a6](https://github.com/Aleph-Alpha/pharia-kernel/commit/da640a62995d9d3892be118bc8412986aab7d136))

### Documentation

- Remove cache endpoints from docs - ([d500267](https://github.com/Aleph-Alpha/pharia-kernel/commit/d50026756b9faeedce55e3e1f340e656fcc65add))

### Builds

- _(deps)_ Bump the minor group with 3 updates - ([6902d38](https://github.com/Aleph-Alpha/pharia-kernel/commit/6902d381f3ddcd5c591ae2e859d852bdd7a4e0dd))

## [0.8.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.9...v0.8.0)

### Features

- Expose text cursors in wit world - ([6e39375](https://github.com/Aleph-Alpha/pharia-kernel/commit/6e3937550c3f6c21aa5ce4a67f640481e0f5f77d))
- Expose token usage in wit world - ([d878a75](https://github.com/Aleph-Alpha/pharia-kernel/commit/d878a7533140bb350abd6fc282d0111d579c6486))
- Expose log probs in completion request in wit world - ([5b25f9d](https://github.com/Aleph-Alpha/pharia-kernel/commit/5b25f9da1b9e8ea1962c68e0eee45476aec0858b))

### Fixes

- Revert breaking rename of content to section - ([028f2f7](https://github.com/Aleph-Alpha/pharia-kernel/commit/028f2f768550656f37d6bbc781ba5004ef05ec47))

### Documentation

- Specify scope of csi shell - ([fb0cb0b](https://github.com/Aleph-Alpha/pharia-kernel/commit/fb0cb0b6da5d308b50437c1434d040fd45a77910))

### Builds

- _(deps)_ Bump the minor group with 6 updates - ([c8013b6](https://github.com/Aleph-Alpha/pharia-kernel/commit/c8013b6fbbc7afb4dbe60a3178c4462afd1b04c2))
- _(deps)_ Bump the minor group across 1 directory with 7 updates - ([66eae10](https://github.com/Aleph-Alpha/pharia-kernel/commit/66eae1015d584fa8c360833d7e21c29a7bde794f))
- _(deps)_ Update aleph-alpha-client to 0.19.0 - ([cf20c00](https://github.com/Aleph-Alpha/pharia-kernel/commit/cf20c009223419a0cb8ae47abb193f05191fbcd9))

## [0.7.9](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.8...v0.7.9)

### Features

- Reflect new log prob structure in wit world - ([9650195](https://github.com/Aleph-Alpha/pharia-kernel/commit/96501953fe2e58e0fa557d8e2bb593d270259ea0))
- Expose log probs in wit world - ([84dd4f1](https://github.com/Aleph-Alpha/pharia-kernel/commit/84dd4f1ecb260b2adc2352950affecbf6c6ee582))

### Fixes

- Handle uppercase roles from v0_2 - ([8bc3862](https://github.com/Aleph-Alpha/pharia-kernel/commit/8bc3862ef37fa2781c2167389ac033486b10d1cc))
- V0.2 chat request in csi - ([efd5985](https://github.com/Aleph-Alpha/pharia-kernel/commit/efd5985303af9a6cf2a24a0d3be917d621e149f2))
- Introduce v0.2 language request - ([4882023](https://github.com/Aleph-Alpha/pharia-kernel/commit/48820230036a292cee31b73adbf8b9750b8f325b))

## [0.7.8](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.7...v0.7.8)

### Features

- Support more languages for select-language in v0.3 WIT world - ([8efa4ac](https://github.com/Aleph-Alpha/pharia-kernel/commit/8efa4ac9b72a25068b8daf7ace018a07556daf71))
- Frequency and presence penalty for completion - ([e5fdb20](https://github.com/Aleph-Alpha/pharia-kernel/commit/e5fdb2083b6b67ad35086a32b9094799210bd3fe))
- Add frequency and presence penalaty to chat params - ([c6cb207](https://github.com/Aleph-Alpha/pharia-kernel/commit/c6cb207bb43d78428e8a1296efe52041f79dd61c))

### Fixes

- Missing comma in wit world - ([e251ae6](https://github.com/Aleph-Alpha/pharia-kernel/commit/e251ae6a0d2b7f64c667205040189300cb4a13e2))

### Builds

- _(deps)_ Bump the minor group across 1 directory with 8 updates - ([af91400](https://github.com/Aleph-Alpha/pharia-kernel/commit/af914005462cab6cbe7acf3ad58c9b3c59d605d0))

## [0.7.7](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.6...v0.7.7)

### Features

- Validate skill metadata schema - ([97d117e](https://github.com/Aleph-Alpha/pharia-kernel/commit/97d117e09c43579a6a6ed27f996faeacd8ef6a10))
- Invoke metadata function from component - ([568b033](https://github.com/Aleph-Alpha/pharia-kernel/commit/568b0330d1f44cff7e9dab571d61a1c49f6affb7))
- Add skill metadata in WIT definition - ([6bbcb3f](https://github.com/Aleph-Alpha/pharia-kernel/commit/6bbcb3fb9d129ec749a5009cfeaf366d5bf42441))

### Documentation

- Add whitespace between shell arguments - ([e3c0ffc](https://github.com/Aleph-Alpha/pharia-kernel/commit/e3c0ffc9a9e28170a1fcee6a78d590bc43eb3d19))
- Fix casing in operating docs - ([f738a6c](https://github.com/Aleph-Alpha/pharia-kernel/commit/f738a6cadb01fbd2f08e903aedd2e69589bd2efc))
- Remove skill development section from operating doc - ([247f2f0](https://github.com/Aleph-Alpha/pharia-kernel/commit/247f2f0dc2fcd57a46d5c670edaf022b139ecadd))
- Return 200 status code if metadata is not implemented - ([e34eb7a](https://github.com/Aleph-Alpha/pharia-kernel/commit/e34eb7aa4830dbac5b86075c4414e471924bea18))

### Builds

- _(deps)_ Bump the minor group with 4 updates - ([147043b](https://github.com/Aleph-Alpha/pharia-kernel/commit/147043b47e048e0f0894f5cd0569afeb3188b3ad))
- _(deps)_ Bump derive_more from 1.0.0 to 2.0.1 - ([d98a1d8](https://github.com/Aleph-Alpha/pharia-kernel/commit/d98a1d89b646fc78c87b7aa84f791e3d03ca59f0))
- _(deps)_ Bump the minor group with 7 updates - ([babfd16](https://github.com/Aleph-Alpha/pharia-kernel/commit/babfd16229ffe0b987b254e44959bee9c60c7557))
- _(deps)_ Bump the minor group across 1 directory with 10 updates - ([ad9de3a](https://github.com/Aleph-Alpha/pharia-kernel/commit/ad9de3a6f13f67626fed09cf8ca832d29fe76b92))

## [0.7.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.5...v0.7.6)

### Features

- Load namespace descriptions concurrently - ([a6106f2](https://github.com/Aleph-Alpha/pharia-kernel/commit/a6106f22b696806f99c696da3fb6af0a4a2a575a))

### Fixes

- Do not retry loading of namespace description - ([8be2930](https://github.com/Aleph-Alpha/pharia-kernel/commit/8be2930c187d0f989c13f7cceaa9ff7eeee271df))

## [0.7.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.4...v0.7.5)

### Features

- Make language selection concurrent in WIT 0.3 - ([35c2a3a](https://github.com/Aleph-Alpha/pharia-kernel/commit/35c2a3ae2b7160af4a8340ffb2caf68ce11974f7))
- Move search in 0.3 WIT to be concurrent - ([fda99df](https://github.com/Aleph-Alpha/pharia-kernel/commit/fda99dff731bd1a3bb745c64f1839efa025ac83d))
- Add metrics for skill execution duration - ([7fe7645](https://github.com/Aleph-Alpha/pharia-kernel/commit/7fe7645e11891cb7baef99f41bcd7287b5ecda27))
- Expose skill execution metric - ([518c17b](https://github.com/Aleph-Alpha/pharia-kernel/commit/518c17b9d78abb2048e5f66ad0e1c0bb77555215))
- Make chunk requests concurrent in v0.3 WIT world - ([e85f36d](https://github.com/Aleph-Alpha/pharia-kernel/commit/e85f36da9d362751c90ffd6c683c126a9252ef6d))
- Make chat requests concurrent in v0.3 wit world - ([9e457cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/9e457cbd996b7d09a9c82c5105b9c83ce35297f3))

### Builds

- _(deps)_ Bump the minor group across 1 directory with 13 updates - ([484473d](https://github.com/Aleph-Alpha/pharia-kernel/commit/484473d911ddd67a3a3f5155bbfde7435062c6ff))

## [0.7.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.3...v0.7.4)

### Fixes

- Actually use correct permission to check authorization, not just existence of the token - ([0bdd2c7](https://github.com/Aleph-Alpha/pharia-kernel/commit/0bdd2c79fe7d644630dc6628d2fe978abe2643c4))

## [0.7.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.2...v0.7.3)

### Features

- Stabilize document-metadata in 2.10 wit world - ([cc074b6](https://github.com/Aleph-Alpha/pharia-kernel/commit/cc074b6e53d0446343ed7881094ff4fdef1fcdbf))
- Add document function to 2.10 wit world - ([f697f10](https://github.com/Aleph-Alpha/pharia-kernel/commit/f697f1065dac0be90f1e0152169df7725288b01f))
- Expose document function in v3 csi shell - ([170daf9](https://github.com/Aleph-Alpha/pharia-kernel/commit/170daf9811fe8b949a7d8213c5c43c85ba300803))
- Do not return option on document request - ([c3b2624](https://github.com/Aleph-Alpha/pharia-kernel/commit/c3b26245d39fb40f93a3b74d52ace75353fa067a))
- Add documents function to 0.3 wit world - ([6718eae](https://github.com/Aleph-Alpha/pharia-kernel/commit/6718eaebd958725d34eafb2bce9bcf6d910b71d0))

### Documentation

- Cleanup outdated docs - ([738efed](https://github.com/Aleph-Alpha/pharia-kernel/commit/738efeded5a44f7fe5da285d444ec005add66248))

### Builds

- _(deps)_ Bump the minor group with 6 updates - ([8c7adae](https://github.com/Aleph-Alpha/pharia-kernel/commit/8c7adaebec273ec408569339af30a6895e1a694c))
- _(deps)_ Bump the minor group with 37 updates - ([c5dccba](https://github.com/Aleph-Alpha/pharia-kernel/commit/c5dccba5c9e02e4130a49b411df02c6711fb5bc4))
- _(deps)_ Bump the wasmtime group with 15 updates - ([debd8ba](https://github.com/Aleph-Alpha/pharia-kernel/commit/debd8ba147fa6f275e40895bb1a2e21dc2051c3a))

## [0.7.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.1...v0.7.2)

### Features

- Remove auth for index route - ([a6c708d](https://github.com/Aleph-Alpha/pharia-kernel/commit/a6c708d77e373f4b49ecf20711f1f2c078aae144))

### Documentation

- Happy 2025 - ([ca675bf](https://github.com/Aleph-Alpha/pharia-kernel/commit/ca675bf31b2b16506fb0637f3a182ea48b9c3014))

### Builds

- _(deps)_ Bump the minor group with 5 updates - ([3cc6c61](https://github.com/Aleph-Alpha/pharia-kernel/commit/3cc6c612f9986b5ad7a20058f2e6ac82425c663c))
- _(deps)_ Text-splitter 0.22 - ([6f8b518](https://github.com/Aleph-Alpha/pharia-kernel/commit/6f8b5181b5f104f9bb8c6b18b33e80d8667deb86))
- _(deps)_ Bump the minor group with 4 updates - ([6713297](https://github.com/Aleph-Alpha/pharia-kernel/commit/67132973557dd8ab501d1ab8808be3ceb0909638))
- Remove docs from container - ([e8e038f](https://github.com/Aleph-Alpha/pharia-kernel/commit/e8e038fd6bb11b43a5af05c20c46b99a3720f326))

## [0.7.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.7.0...v0.7.1)

### Features

- Make complete parallel in 0.3 wit world - ([e6d7a11](https://github.com/Aleph-Alpha/pharia-kernel/commit/e6d7a11591288367514bcaeed180282f8313a265))
- Push parallel document metadata to 0.3 wit world - ([e3aa9b3](https://github.com/Aleph-Alpha/pharia-kernel/commit/e3aa9b3849b33f71efc8afff8725c4027d4bc618))

### Fixes

- Expose parallel search metadata interface via csi shell - ([f6dc770](https://github.com/Aleph-Alpha/pharia-kernel/commit/f6dc770a445bf273b57313aa8634639d29a4d898))
- Only trace number of parallel document metadata requests - ([f1672a2](https://github.com/Aleph-Alpha/pharia-kernel/commit/f1672a249ce316c86c8f0a7ee6471049fa3b9711))

### Documentation

- Add release process - ([eae95ea](https://github.com/Aleph-Alpha/pharia-kernel/commit/eae95ea3af2e96fc3dc4f49f96232d8c0b613b3e))

### Builds

- _(deps)_ Bump the minor group with 34 updates - ([849b80c](https://github.com/Aleph-Alpha/pharia-kernel/commit/849b80c9540e1208b20da1d1d1b74dce349e25a7))
- _(deps)_ Bump the minor group with 2 updates - ([14fd7db](https://github.com/Aleph-Alpha/pharia-kernel/commit/14fd7db14e5b74315e4bcd62a41ca8fbb5f9da15))
- _(deps)_ Bump the minor group with 14 updates - ([f3a5266](https://github.com/Aleph-Alpha/pharia-kernel/commit/f3a5266f102d4cb2970798650fcd4277f87ec6cb))
- _(deps)_ Bump the minor group with 5 updates - ([9360c1c](https://github.com/Aleph-Alpha/pharia-kernel/commit/9360c1c0a1cb60d46598c431bae2102d53afa3b6))

## [0.7.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.6.0...v0.7.0)

### Features

- Add complete-return-special-tokens function to wit world - ([7defb90](https://github.com/Aleph-Alpha/pharia-kernel/commit/7defb90a8d86f80b9c8a4e73ba2285e1ae221a6b))
- Support v3 requests in csi shell - ([daa16d0](https://github.com/Aleph-Alpha/pharia-kernel/commit/daa16d090ffaa9086e28d1e81fdbadf916e8c3b0))
- Introduce pharia:skill@0.3.0-alpha.1 - ([8f6b22e](https://github.com/Aleph-Alpha/pharia-kernel/commit/8f6b22efcede3f89c6404f9eb42d3b958584480b))
- Make namespace pattern show up in docs - ([2d2d03d](https://github.com/Aleph-Alpha/pharia-kernel/commit/2d2d03dc527f6df007f81777df1fd54672c5b383))
- Validate skill requests into namespace new type - ([926fc44](https://github.com/Aleph-Alpha/pharia-kernel/commit/926fc445d4955870edc05fedcdbb0dfe6b8943a7))

### Fixes

- Add back since gates - ([cbea6f4](https://github.com/Aleph-Alpha/pharia-kernel/commit/cbea6f4090594e59e9d3345d411442674e232489))
- Don't swallow error messages from skill creation - ([5b6d557](https://github.com/Aleph-Alpha/pharia-kernel/commit/5b6d557b1ff8c0b54d506d933f4cd4182ba5edb3))

## [0.6.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.5.0...v0.6.0)

### Features

- Update index html about what kernel is - ([b6f291f](https://github.com/Aleph-Alpha/pharia-kernel/commit/b6f291fca631d4d9d2ed687bef555d3676b051f7))
- [**breaking**] Streamline naming of environment variables - ([d22f999](https://github.com/Aleph-Alpha/pharia-kernel/commit/d22f999b546d8d45fef440eeedbcefbfa69cdfed))
- [**breaking**] Rename operator-config file to just config - ([999a784](https://github.com/Aleph-Alpha/pharia-kernel/commit/999a784e62e08471be0f607a7585593f310520a8))
- Support loading config from file and env variables - ([0c8286d](https://github.com/Aleph-Alpha/pharia-kernel/commit/0c8286dfa64339cdd2648bb1af98eab96f9421c6))
- Limit valid namespace names to ascii chars, digits and hyphens - ([b8b3705](https://github.com/Aleph-Alpha/pharia-kernel/commit/b8b3705ff49cea91cc14c9cf644a106cb750e2c9))
- Reject empty namespace names - ([26554f9](https://github.com/Aleph-Alpha/pharia-kernel/commit/26554f9bd4376d58f98ce6c5bc17c587033f7528))
- Restrict namespace length to 64 chars - ([8ae2c10](https://github.com/Aleph-Alpha/pharia-kernel/commit/8ae2c107a160e8cf815c88ed11532229805029db))
- Validate namespace name is specified in kebab-case - ([5860525](https://github.com/Aleph-Alpha/pharia-kernel/commit/5860525062987086cc68ab28321c30458ade8993))

### Documentation

- Pharia ai token is not needed to run kernel locally - ([6e5755c](https://github.com/Aleph-Alpha/pharia-kernel/commit/6e5755c3b1c9f33a4a8d19d4a51df7ca3e73056d))

### Builds

- _(deps)_ Bump the minor group with 8 updates - ([49c3237](https://github.com/Aleph-Alpha/pharia-kernel/commit/49c32377f97a9c364e695baad8bd30c436093579))
- _(deps)_ Bump the minor group with 7 updates - ([cd86400](https://github.com/Aleph-Alpha/pharia-kernel/commit/cd86400a3475fb142a66a31ec2c371744217fe08))
- _(deps)_ Bump tempfile from 3.14.0 to 3.15.0 in the minor group - ([936aeba](https://github.com/Aleph-Alpha/pharia-kernel/commit/936aeba6106a0e026ba6a42185472c37b4307fb4))

## [0.5.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.14...v0.5.0)

### Features

- Remove fallback to "dev" namespace for operator-config.toml - ([e0933df](https://github.com/Aleph-Alpha/pharia-kernel/commit/e0933dfd308d0543f29c61958966850cf42a9460))
- Prioritize oci registry over file registry - ([79b37dd](https://github.com/Aleph-Alpha/pharia-kernel/commit/79b37dd71a4767700b853a8f914d5ec7565aa271))
- [**breaking**] Flatten repository in operator config - ([a46cabc](https://github.com/Aleph-Alpha/pharia-kernel/commit/a46cabc9b6d746e94390bc4714250035bed1e17e))
- Use untagged enum for registry - ([972a515](https://github.com/Aleph-Alpha/pharia-kernel/commit/972a515745e009555f3e42f4f9b4e31c78ca28b7))
- [**breaking**] Expect operator config variables in kebab-case - ([d0b5c00](https://github.com/Aleph-Alpha/pharia-kernel/commit/d0b5c00475e1bee0499f59ccad28cf73f4da9514))
- Handle base repository with leading/trailing slashes or empty content - ([37760fb](https://github.com/Aleph-Alpha/pharia-kernel/commit/37760fb63d34cb45be4500043f427fd89a11f9be))
- Support kebab-case and snake_case for fields in namespace config - ([671a173](https://github.com/Aleph-Alpha/pharia-kernel/commit/671a173770e6ece3b956e13d4bf92b02a25da3ca))
- [**breaking**] Rename registry name field as `name` - ([3cf82f9](https://github.com/Aleph-Alpha/pharia-kernel/commit/3cf82f9cc0d429265b338921375c84d24e7d6aac))
- [**breaking**] Update fields for credentials to take values directly - ([0ab1f29](https://github.com/Aleph-Alpha/pharia-kernel/commit/0ab1f29d4dc92106a65e6c12a769e100592b92d2))
- Support configuring namespaces via environment variables - ([6319105](https://github.com/Aleph-Alpha/pharia-kernel/commit/631910573df833bf87bc43c5636601961d3dce0d))

### Documentation

- Update to new namespace config definition - ([7c746d7](https://github.com/Aleph-Alpha/pharia-kernel/commit/7c746d7b3488ca23794f1898f69ceccdfa73b416))
- Remove outdated comment - ([06017b4](https://github.com/Aleph-Alpha/pharia-kernel/commit/06017b482c25b5bf110a938cd0844f3d2f258401))
- Environment config source - ([7a154e6](https://github.com/Aleph-Alpha/pharia-kernel/commit/7a154e6bc4a9844ac9a00fce27d452bda20233db))

### Builds

- _(deps)_ Bump the minor group with 6 updates - ([213aa5a](https://github.com/Aleph-Alpha/pharia-kernel/commit/213aa5a874c403cb798bacb8841478b406124803))
- _(deps)_ Bump the minor group with 4 updates - ([dc99868](https://github.com/Aleph-Alpha/pharia-kernel/commit/dc998685b287155d0fba92e95ef9d3ebb35e6f46))
- _(deps)_ Bump the wasmtime group with 15 updates - ([01ff0fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/01ff0fdf7f38ef00f24d86ada81178e55f2798ce))
- _(deps)_ Bump the minor group with 7 updates - ([6c6df44](https://github.com/Aleph-Alpha/pharia-kernel/commit/6c6df442213578d204542e161d9402dcb9f44548))
- _(deps)_ Bump unicase from 2.8.0 to 2.8.1 in the minor group - ([9e89c51](https://github.com/Aleph-Alpha/pharia-kernel/commit/9e89c5192092e3779bb0e91c1241cb5d937d3f94))
- _(deps)_ Bump the minor group with 9 updates - ([3e60602](https://github.com/Aleph-Alpha/pharia-kernel/commit/3e60602202af6673dcc3b7301d239eb88116401f))

## [0.4.14](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.13...v0.4.14)

### Features

- Hide docs and index behind auth middleware - ([d46d130](https://github.com/Aleph-Alpha/pharia-kernel/commit/d46d1300a71a23f34c3d3d70f7e1ddcca151f5b5))

### Fixes

- Url encode documenent name when requesting metadata - ([d1da157](https://github.com/Aleph-Alpha/pharia-kernel/commit/d1da157f49fd9dfee97855f8449dce1c56d738ee))

### Documentation

- Add comment about indexes not needing url encoding - ([5784e9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/5784e9ae7d7e778226434cf180dbbd28a8c099e9))

### Builds

- _(deps)_ Bump the minor group with 5 updates - ([57d2588](https://github.com/Aleph-Alpha/pharia-kernel/commit/57d258846d6733ed9e63b1c96748737a910c6b09))

## [0.4.13](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.12...v0.4.13)

### Documentation

- Typo - ([2d952fc](https://github.com/Aleph-Alpha/pharia-kernel/commit/2d952fc783e1136da2ab881d210f967307dd4ed0))

## [0.4.12](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.11...v0.4.12)

### Features

- Better error message for unsupported functions - ([bf38195](https://github.com/Aleph-Alpha/pharia-kernel/commit/bf381952c07c111d6f574a55943e7fb9e575bc4b))
- Better error message for invalid version strings - ([6362133](https://github.com/Aleph-Alpha/pharia-kernel/commit/63621339ff80149bf4ad81b33f911cd81e552a20))
- Improve error messages for unsupported skill versions - ([d3ca9b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/d3ca9b0c97a1775d566b77c956332679c90e3921))

### Fixes

- Use max to get latest version - ([cec26cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/cec26cb34d78e6612302cdda3736d84159896ab1))

### Documentation

- Clearer error message for unknown function - ([8d3f2d2](https://github.com/Aleph-Alpha/pharia-kernel/commit/8d3f2d2fdd1b3e07273fb9c5ece2717c59cbe102))
- Add more comments for version parsing logic - ([b439fc3](https://github.com/Aleph-Alpha/pharia-kernel/commit/b439fc33d665fe5addaa37fdc7a98136c2245507))
- Add comments for version parsing logic - ([f62f453](https://github.com/Aleph-Alpha/pharia-kernel/commit/f62f453b687eb901a8aaca2b48c639bd6307fad0))

## [0.4.10](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.9...v0.4.10)

### Documentation

- Typo - ([21ffa54](https://github.com/Aleph-Alpha/pharia-kernel/commit/21ffa54b450d6600add7038e6b2c26ae06495659))

## [0.4.8](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.7...v0.4.8)

### Fixes

- Specify search introduction with `since` in wit world - ([ab827b3](https://github.com/Aleph-Alpha/pharia-kernel/commit/ab827b327eceffdb489f20df67b76a352738f169))
- Bump wit version because of unstable document-metadata - ([eb8c322](https://github.com/Aleph-Alpha/pharia-kernel/commit/eb8c322b8de63c508ee6610f3a01ec4b157869a9))

## [0.4.6](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.5...v0.4.6)

### Features

- Use unversioned for document_metadata - ([dc06e90](https://github.com/Aleph-Alpha/pharia-kernel/commit/dc06e9003dd30fee8e0813bdb26a097fc0f008b6))
- Document_metadata in the wit world and test via skill - ([c15e91d](https://github.com/Aleph-Alpha/pharia-kernel/commit/c15e91d2e1f166fc9782feb4446bdc195c5b308b))
- Add document_metadata to csi - ([a0fc362](https://github.com/Aleph-Alpha/pharia-kernel/commit/a0fc36242db383a5d1bf1a3f385c6da47f7bf14c))
- Introduct document_metadata in CsiForSkills - ([84ca64d](https://github.com/Aleph-Alpha/pharia-kernel/commit/84ca64d8427c8e59aaaead1651d0120136005256))
- Document_metadata in CSI - ([c61c855](https://github.com/Aleph-Alpha/pharia-kernel/commit/c61c855697401675400cee1069a5d65c3ad5eab0))
- Search actor can request metadata - ([f531fd2](https://github.com/Aleph-Alpha/pharia-kernel/commit/f531fd285cdb5da9510b86260a22462b2f1f7254))
- Add document metadata retrieval - ([fccd7be](https://github.com/Aleph-Alpha/pharia-kernel/commit/fccd7be086e45c45fa24291d62b03d1e20b682b8))

### Fixes

- Do not convert to string first, however " still remain - ([aaddc59](https://github.com/Aleph-Alpha/pharia-kernel/commit/aaddc5905dd77542a99c3a879ca9117eab25499d))

### Documentation

- Typo - ([8c2441c](https://github.com/Aleph-Alpha/pharia-kernel/commit/8c2441cf0e7ed84e12f481b9c1a906c0263d436e))

### Builds

- _(deps)_ Bump the minor group with 7 updates - ([525469f](https://github.com/Aleph-Alpha/pharia-kernel/commit/525469fc49149bdd20742c7fce36afbd8ba23cc2))
- _(deps)_ Bump the minor group with 3 updates - ([c5327a5](https://github.com/Aleph-Alpha/pharia-kernel/commit/c5327a5c37717d5b1e217d4277a9ee9921b6aad1))
- _(deps)_ Bump the minor group with 5 updates - ([ed232d9](https://github.com/Aleph-Alpha/pharia-kernel/commit/ed232d95353a90bba7cc4bd5b27d840227c1dcd8))
- _(deps)_ Bump aleph-alpha-client in the minor group - ([97c44ac](https://github.com/Aleph-Alpha/pharia-kernel/commit/97c44acab321165d2aed61414fdb63e22098084b))

## [0.4.5](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.4...v0.4.5)

### Documentation

- Update comments - ([0d69a6e](https://github.com/Aleph-Alpha/pharia-kernel/commit/0d69a6eefce7f0e42d68a17d4756ee5a21e0b885))

## [0.4.4](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.3...v0.4.4)

### Features

- Run_skill endpoint invokes skill executor api - ([51d79e9](https://github.com/Aleph-Alpha/pharia-kernel/commit/51d79e9724582c58cb6bc52b0ef4eae3e66188d0))
- Introduce skills run endpoint - ([39b324e](https://github.com/Aleph-Alpha/pharia-kernel/commit/39b324eea5da4ce4956a1606935937ed389af189))
- Add more metrics buckets for request duration - ([e36d800](https://github.com/Aleph-Alpha/pharia-kernel/commit/e36d80003ab5bad6103134fa32af7c7eef303e66))

### Fixes

- Missing auth tokens in api docs - ([4393cde](https://github.com/Aleph-Alpha/pharia-kernel/commit/4393cde4278bfbba956e9917d00640e20d5a912d))

### Documentation

- Add execute_skill endpoint as deprecated - ([4f70203](https://github.com/Aleph-Alpha/pharia-kernel/commit/4f70203ed68cc0fae0899c1d609d095675c8b9a1))
- Remove execute skill from docs, update example for run_skill - ([05cf4fb](https://github.com/Aleph-Alpha/pharia-kernel/commit/05cf4fbcfafcf7e494a926e0deb8d62e5bf103ea))

### Builds

- _(deps)_ Bump the minor group with 8 updates - ([b01512e](https://github.com/Aleph-Alpha/pharia-kernel/commit/b01512eb101627b5f79e52e8104a89e217ab0002))

## [0.4.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.2...v0.4.3)

### Features

- Update default authorization address to Pharia IAM - ([338c2f1](https://github.com/Aleph-Alpha/pharia-kernel/commit/338c2f13faf2ed5a6c5cf83143afa144fca46800))
- Rename env variable to PHARIA_AI_TOKEN - ([ac649a0](https://github.com/Aleph-Alpha/pharia-kernel/commit/ac649a0fb4626cb82d6dc29eed8f215d8d9bec91))

### Builds

- _(deps)_ Bump the minor group with 4 updates - ([968ac0e](https://github.com/Aleph-Alpha/pharia-kernel/commit/968ac0e712c9559cfbc70eeb2df2af9085f6efa1))

## [0.4.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.1...v0.4.2)

### Builds

- _(deps)_ Bump the minor group with 2 updates - ([0a83ead](https://github.com/Aleph-Alpha/pharia-kernel/commit/0a83ead829ad89a076dd3b17c6675cba556ecbd1))

## [0.4.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.4.0...v0.4.1)

### Features

- Prefix all metric names - ([b0583cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/b0583cbafff84d90ab73736f2ed7fb3bd89ce96b))

## [0.4.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.3.2...v0.4.0)

### Features

- [**breaking**] Execute_skill and drop_cached_skill require namespace with skill name - ([d776680](https://github.com/Aleph-Alpha/pharia-kernel/commit/d77668066b88e8c1b0c8235d055fdbbab6f0d7a6))
- [**breaking**] Remove support for unversioned and 0.1 wit worlds - ([f3b5c82](https://github.com/Aleph-Alpha/pharia-kernel/commit/f3b5c82e0a1132b1cf178898886616f8450f21be))

### Documentation

- Apply seasonal theme - ([d6d6107](https://github.com/Aleph-Alpha/pharia-kernel/commit/d6d61070ec8456e0f64e3b96c8d1bb37cc21e899))

### Builds

- _(deps)_ Bump the minor group with 18 updates - ([46e094b](https://github.com/Aleph-Alpha/pharia-kernel/commit/46e094b4ae28117d63d00b922fd69f9325180380))

## [0.3.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.3.1...v0.3.2)

### Features

- Sort skills by namespace and name - ([371ad48](https://github.com/Aleph-Alpha/pharia-kernel/commit/371ad48e7943619bc91ad0abecbd020dc953056c))

### Builds

- _(deps)_ Bump the minor group across 1 directory with 19 updates - ([ded5b89](https://github.com/Aleph-Alpha/pharia-kernel/commit/ded5b895209c595d9bade684ca422977f89e9163))

## [0.3.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.3.0...v0.3.1)

### Features

- Migrate health endpoint - ([9c23321](https://github.com/Aleph-Alpha/pharia-kernel/commit/9c23321f252697f65f8ac3f37553a9f30f188e03))
- Skill request cache ensures skill store only has one pending request per skill - ([2fe288f](https://github.com/Aleph-Alpha/pharia-kernel/commit/2fe288f0b59e90b657a044b4d243cef894ca47c9))

## [0.3.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.2.3...v0.3.0)

### Features

- Add authorization_addr endpoint to configure where to check authorization requests from kernel - ([76b9c44](https://github.com/Aleph-Alpha/pharia-kernel/commit/76b9c4458e4b0752df8c7e3267360bf52c09474d))
- Implement authorization middleware with actor model - ([0991fe6](https://github.com/Aleph-Alpha/pharia-kernel/commit/0991fe63083f5e5811c1b7960abd5db0dfe78f2a))
- Require token on CSI and skill routes - ([3718963](https://github.com/Aleph-Alpha/pharia-kernel/commit/37189636bb0df405ebd06110a0d7a11a37f8a59f))
- Skill loading happens in background - ([b0b533a](https://github.com/Aleph-Alpha/pharia-kernel/commit/b0b533a8a414b0d7986e29b96dbf369ce7c3e32e))
- Expose metrics for total CSI function calls - ([2b1fd3b](https://github.com/Aleph-Alpha/pharia-kernel/commit/2b1fd3b1f0c21b4c348bebbb6a1060bd6a9428c7))
- Allow metrics addr to be configurable - ([3fce17c](https://github.com/Aleph-Alpha/pharia-kernel/commit/3fce17c30d5dca0f0f68e32e90267a82d7f17840))
- Expose metrics endpoint at port 9000 - ([e42c931](https://github.com/Aleph-Alpha/pharia-kernel/commit/e42c931e36ca591ae6d42514f63579d6d6b67cb2))
- Emit library-level metrics from shell - ([c36e7c6](https://github.com/Aleph-Alpha/pharia-kernel/commit/c36e7c6f538916725abe5a1612ffb2bcb42e14b4))

### Fixes

- Always send response in message channel - ([b5b0fb0](https://github.com/Aleph-Alpha/pharia-kernel/commit/b5b0fb0418ba0ef842168353b6c25702997c6990))

### Documentation

- Fix multiple typos - ([33a5b56](https://github.com/Aleph-Alpha/pharia-kernel/commit/33a5b563c6b956e00ffbb91bdac65356f311bea2))

### Builds

- _(deps)_ Bump the minor group with 15 updates - ([9508430](https://github.com/Aleph-Alpha/pharia-kernel/commit/95084308131bd1d806d2b73351e45f7e1764a029))
- _(deps)_ Bump the wasmtime group with 15 updates - ([a0a878a](https://github.com/Aleph-Alpha/pharia-kernel/commit/a0a878a619c5e9168647dbca76d9a21bbb922e24))
- _(deps)_ Bump rustls from 0.23.17 to 0.23.18 in the cargo group - ([5175d3f](https://github.com/Aleph-Alpha/pharia-kernel/commit/5175d3fee79f2cda7448379c4ab226117990340d))

## [0.2.3](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.2.2...v0.2.3)

### Builds

- _(deps)_ Bump the minor group with 8 updates - ([f0178e4](https://github.com/Aleph-Alpha/pharia-kernel/commit/f0178e4fcc9d9f497f564ce3ebae5c73216299a2))
- _(deps)_ Bump the minor group with 2 updates - ([1723639](https://github.com/Aleph-Alpha/pharia-kernel/commit/1723639589ac6d2c6309ab281ee9f0398bfeab67))
- _(deps)_ Bump the minor group across 1 directory with 11 updates - ([04466b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/04466b03015fec77fd795c3ca9e9907fb5e691e0))
- _(deps)_ Bump the minor group with 5 updates - ([c97cf9e](https://github.com/Aleph-Alpha/pharia-kernel/commit/c97cf9ef41bf0486bd5ca9bc654fe60e21f22a7f))

## [0.2.2](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.2.1...v0.2.2)

### Features

- Allow cors requests for frontend apps - ([907061d](https://github.com/Aleph-Alpha/pharia-kernel/commit/907061df37ce90a94a86c6b7ab6857ea82e45897))

### Builds

- _(deps)_ Bump the minor group with 10 updates - ([99fb63c](https://github.com/Aleph-Alpha/pharia-kernel/commit/99fb63cb07f57909c3c9caf0300e6853bb9e3c4c))
- _(deps)_ Bump the minor group with 6 updates - ([59527d1](https://github.com/Aleph-Alpha/pharia-kernel/commit/59527d172baa1bfe98c9dd3e8491e164d18a8ac7))

## [0.2.0](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.1.1...v0.2.0)

### Features

- [**breaking**] Load skill registry auth per namespace from env - ([91aa292](https://github.com/Aleph-Alpha/pharia-kernel/commit/91aa2920bddf43faad339fbabcbd46611bbdce44))

### Documentation

- Specify purpose of associated registry method - ([d884c04](https://github.com/Aleph-Alpha/pharia-kernel/commit/d884c04d5c52300ef5b3e5bdfeefdf911f6fe07e))
- Specify purpose of OperatorConfig::local - ([25847ea](https://github.com/Aleph-Alpha/pharia-kernel/commit/25847ea5eccc9676333d17d00d4bfbeecbdd8b4b))
- Fix typos - ([3abf7f1](https://github.com/Aleph-Alpha/pharia-kernel/commit/3abf7f17684570107ce034582bd6e53b3cb2c11e))
- Specify env vars for registry access - ([adefba8](https://github.com/Aleph-Alpha/pharia-kernel/commit/adefba8d2ddc890ad53d5f9afcb94345c658e46f))
- Update usages on Pharia Skill CLI - ([32afa3e](https://github.com/Aleph-Alpha/pharia-kernel/commit/32afa3eeb94bb527694803fbbe5c2fb3538c668f))

### Builds

- _(deps)_ Bump the minor group with 4 updates - ([5b5175c](https://github.com/Aleph-Alpha/pharia-kernel/commit/5b5175cbb994e896d986b0c328478edebe0ee6b3))

## [0.1.1](https://github.com/Aleph-Alpha/pharia-kernel/compare/v0.1.0...v0.1.1)

### Builds

- _(deps)_ Bump thiserror from 1.0.68 to 2.0.0 - ([5644b23](https://github.com/Aleph-Alpha/pharia-kernel/commit/5644b234da0a343940cbd4242e2c729e4bd2a509))
- _(deps)_ Bump appany/helm-oci-chart-releaser from 0.3.0 to 0.4.2 - ([683c028](https://github.com/Aleph-Alpha/pharia-kernel/commit/683c028a897eb0521bc22a7b3c738e3ce5879295))
- _(deps)_ Bump the minor group across 1 directory with 14 updates - ([957ebaf](https://github.com/Aleph-Alpha/pharia-kernel/commit/957ebaf24b944d9a79ebbb08939d008bf9e1278d))
- _(deps)_ Bump wasmtime from 26.0.0 to 26.0.1 in the cargo group - ([d82ff04](https://github.com/Aleph-Alpha/pharia-kernel/commit/d82ff04ffcf43db5657b6465e4efbba7e868d706))

## [0.1.0]

### Features

- Add Pharia Kernel Helm chart - ([16297ec](https://github.com/Aleph-Alpha/pharia-kernel/commit/16297ec4bdde755506cc2c0c32d99d3e369900bf))
- Allow setting using pooling allocation from env - ([5f13de6](https://github.com/Aleph-Alpha/pharia-kernel/commit/5f13de608d686786540ca1800d1313dc7897768d))
- Enable pooling allocation when possible - ([718f029](https://github.com/Aleph-Alpha/pharia-kernel/commit/718f029f7d039a76a6411b035912254f49a34bf5))
- Add kernel path argument to specify pharia kernel binary - ([05ce6a4](https://github.com/Aleph-Alpha/pharia-kernel/commit/05ce6a4fd9f89bdec37b0f8bf8498751cb9b7c16))
- Add Rust skills - ([b1db661](https://github.com/Aleph-Alpha/pharia-kernel/commit/b1db661f1c039cc8f5759368a6069ff89491d954))
- Add Rust skill to resource evaluation - ([a3f296d](https://github.com/Aleph-Alpha/pharia-kernel/commit/a3f296dcb69b89c44a5fefd0e38b00a33457ff0b))
- Add delete_skill command - ([92f68c2](https://github.com/Aleph-Alpha/pharia-kernel/commit/92f68c224159b3af9cee30d6f7d0a1c829933c25))
- Add drop_cached_skill command and organize cmds scripts - ([14e2abe](https://github.com/Aleph-Alpha/pharia-kernel/commit/14e2abec8f911fb6eb6785721c1db249d378909c))
- Add machine information logging - ([90e0d9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/90e0d9a448bf779a18f97076e17f3feb09406568))
- Evaluate memory consumption of python skills in Pharia Kernel - ([e144d1a](https://github.com/Aleph-Alpha/pharia-kernel/commit/e144d1a123dfbecaa58704139219372c4407df86))
- Update drop cached skill endpoint path - ([ed84d61](https://github.com/Aleph-Alpha/pharia-kernel/commit/ed84d61d4a215b027a466595db885eec21a8a747))
- Expose chat via http csi - ([3af889e](https://github.com/Aleph-Alpha/pharia-kernel/commit/3af889e34844fa89327f1b0ab7e3a37b5df852cb))
- Add retry logic for chat request - ([21c6ed1](https://github.com/Aleph-Alpha/pharia-kernel/commit/21c6ed176a151f82ab560dad27a8c905b21b0a84))
- Chat request invokes inference client - ([6cde125](https://github.com/Aleph-Alpha/pharia-kernel/commit/6cde1256f4797e456421db8cf39c7fa76543f53e))
- Add dummy chat endpoint to wit world - ([7fec413](https://github.com/Aleph-Alpha/pharia-kernel/commit/7fec413a4de65d53153f0258a26dca906e81cb0a))
- Add basic styling to the index page - ([81e1369](https://github.com/Aleph-Alpha/pharia-kernel/commit/81e136954adebde1758a789939d6c275223ad3ed))
- Make search public in wit world - ([b7fd97c](https://github.com/Aleph-Alpha/pharia-kernel/commit/b7fd97c6f2730cb4be223d228eda6ba152f71870))
- Add index.html - ([21b2d2b](https://github.com/Aleph-Alpha/pharia-kernel/commit/21b2d2b0be8137966e3bffd237f48e4c5d467418))
- Add search to remote CSI - ([dc33f80](https://github.com/Aleph-Alpha/pharia-kernel/commit/dc33f805380272296d95cf0f520bc7b76d508f31))
- Remove csi shell on different port - ([53af076](https://github.com/Aleph-Alpha/pharia-kernel/commit/53af07657a3ef11f323b510b93c63836eef74fb1))
- Shell provides csi endpoint - ([d2c6c43](https://github.com/Aleph-Alpha/pharia-kernel/commit/d2c6c437566b9ce58f969d8ef8327b1b9dd48207))
- Update syntax for version value in CSI shell - ([6c39871](https://github.com/Aleph-Alpha/pharia-kernel/commit/6c39871013d9cf21381be24b7de91e091e44cfa8))
- Implement full CSI v0.2 - ([2c05c66](https://github.com/Aleph-Alpha/pharia-kernel/commit/2c05c66d428b6e254869b2669baa011cf8b6f1e5))
- Use snake case for finish reason serialization - ([0f28f2d](https://github.com/Aleph-Alpha/pharia-kernel/commit/0f28f2de821dcbe2a54935159211c78cedbd7c9b))
- Use inference for CSI completion request - ([1e22f79](https://github.com/Aleph-Alpha/pharia-kernel/commit/1e22f79792ab7adc5b3c25d463f278b6e297cc46))
- Expose csi endpoint for dummy completion - ([dd90f00](https://github.com/Aleph-Alpha/pharia-kernel/commit/dd90f0043e94319cce2fccaf8080f2d37646b5e2))
- Allow for skills to be executed concurrently - ([e593611](https://github.com/Aleph-Alpha/pharia-kernel/commit/e5936116ef9c176d1926398926a755062513b5c3))
- Handle invalid input error before propagating it - ([ce940f2](https://github.com/Aleph-Alpha/pharia-kernel/commit/ce940f28e50613d95a1550df8afabe546f3eee63))
- Format internal skill error message when converting to anyhow error - ([2d26dcf](https://github.com/Aleph-Alpha/pharia-kernel/commit/2d26dcf675ff41764bfb038738758040777ea1d4))
- Add fallback for log level when processing app config - ([8d05991](https://github.com/Aleph-Alpha/pharia-kernel/commit/8d05991d62c5ae0e50f2d600e0633109d513ed48))
- Allow configuring update interval for polling namespace configurations - ([c858a8f](https://github.com/Aleph-Alpha/pharia-kernel/commit/c858a8f1ce0a2b54f13eef167636a32b2520827b))
- Implement watch mode for namespace - ([a03d152](https://github.com/Aleph-Alpha/pharia-kernel/commit/a03d1526ad31ec43899afce10d55dc8df46b6b69))
- Fall back to local namespace if operator-config.toml is not provided - ([e7e60bc](https://github.com/Aleph-Alpha/pharia-kernel/commit/e7e60bcc20151d3ed710faef351d60c2db44e6c8))
- Instrument execute skill for all csi calls - ([cdd0d1c](https://github.com/Aleph-Alpha/pharia-kernel/commit/cdd0d1c667d7eb1a5f9012744ec82ebd1d1bb472))
- Enable trace layer on request - ([bedb6be](https://github.com/Aleph-Alpha/pharia-kernel/commit/bedb6beb07074b1f7fa83b69049980aee9e7d6e6))
- Only enable OpenTelemetry if the endpoint is provided in .env - ([c6dfb7f](https://github.com/Aleph-Alpha/pharia-kernel/commit/c6dfb7fe93d5f844a98a8ed30b4b729621c70ac8))
- Introduce OpenTelemtry as a tracing subscriber - ([9f2394a](https://github.com/Aleph-Alpha/pharia-kernel/commit/9f2394acc617169a6f8fec2b2b3a2054e768d534))
- Run completions in batch concurrently - ([151d3c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/151d3c822ebaa8e0de85db0159d3c89916f3d522))
- Add complete_all - ([be24cde](https://github.com/Aleph-Alpha/pharia-kernel/commit/be24cdeb9709368829fac0fbc56385f65423c1ce))
- Add trace-level log for selecting language - ([ebcf5ba](https://github.com/Aleph-Alpha/pharia-kernel/commit/ebcf5ba8ef76a4ec01538235f8b7dd081e218242))
- Add select_language implementation to v0.2 host implementation - ([396c20a](https://github.com/Aleph-Alpha/pharia-kernel/commit/396c20a5421b83337914c21032e4a88b8a76db0a))
- Add language selection - ([08889a2](https://github.com/Aleph-Alpha/pharia-kernel/commit/08889a2dfb19c6a2cdd4728b3879666532cf8ec5))
- For operator trace calls to csi complete and chunk - ([fa97afa](https://github.com/Aleph-Alpha/pharia-kernel/commit/fa97afaa88d6b437e64b5f3a7662fd13dd58f763))
- Caching of tokenizers - ([1c5aaf2](https://github.com/Aleph-Alpha/pharia-kernel/commit/1c5aaf27ae6028f0116375f478e95cb748f4a29f))
- Kernel no longer needs API TOKEN to boot up - ([e2cd043](https://github.com/Aleph-Alpha/pharia-kernel/commit/e2cd04397f67b2ff9c5b7f9ed2ab346ea748d4d4))
- Handle error for fetching tokenizer when chunking - ([32150d1](https://github.com/Aleph-Alpha/pharia-kernel/commit/32150d1d8d8c222a2b9d0d88458818e6922f6a73))
- Implement chunk in CSI - ([52803d6](https://github.com/Aleph-Alpha/pharia-kernel/commit/52803d63adde3a2db4c637137f6ead4178ddfce3))
- Overlap is not optional - ([d7c3932](https://github.com/Aleph-Alpha/pharia-kernel/commit/d7c3932b055e70268e940490e1150691bff9bf79))
- Send RemoveInvalidNamespace only if the namespace was marked as invalid - ([16343ef](https://github.com/Aleph-Alpha/pharia-kernel/commit/16343ef9196daeb1770d29f5fc6b4036232b4e5e))
- Mark namespace as invalid for unrecoverable error in namespace config - ([9c59db6](https://github.com/Aleph-Alpha/pharia-kernel/commit/9c59db6473d7c7efa32a9d44230690a8aaa32659))
- Unload skill on unrecoverable namespaceloader error - ([b46cba4](https://github.com/Aleph-Alpha/pharia-kernel/commit/b46cba476d18e01c50a4ae8563888e20ddbd25b5))
- 400 status code for non-existing skills - ([3b97e26](https://github.com/Aleph-Alpha/pharia-kernel/commit/3b97e261bc31fccbbda2066bdeeae528faa569f7))
- Distinguis SkillDoesNotExist error - ([005f7d1](https://github.com/Aleph-Alpha/pharia-kernel/commit/005f7d1c841b73c8bc75d1c6b481b6d7ac794f92))
- Shorten configuration observer update interval - ([240d4fd](https://github.com/Aleph-Alpha/pharia-kernel/commit/240d4fd45c5a55c3926a3fe59ec268798cc338f6))
- Panic on empty inference address from environment - ([d4cc567](https://github.com/Aleph-Alpha/pharia-kernel/commit/d4cc56739c54252a01eb967430b69099390f6697))
- Better error message in case token is missing in environment - ([6cb22f7](https://github.com/Aleph-Alpha/pharia-kernel/commit/6cb22f7a79eea7cbb75a30409f1276740b909703))
- Expect operator-config.toml when launching service - ([e004271](https://github.com/Aleph-Alpha/pharia-kernel/commit/e0042714d3d405a9c1edd2b7e1844aa2864b0abe))
- Make namespace config access token configurable - ([e3f1d59](https://github.com/Aleph-Alpha/pharia-kernel/commit/e3f1d5967d7b5adc749c4ded55d921fb994bf9b0))
- Use atomic upsert operation instead of add and remove - ([271f1a6](https://github.com/Aleph-Alpha/pharia-kernel/commit/271f1a6524ae1777f6a5d081f4ca9dffaf2ddc23))
- Sleep at most update_interval in configuration observer - ([c84bc7d](https://github.com/Aleph-Alpha/pharia-kernel/commit/c84bc7df37d9a20df93519c7b362acd9df874b8f))
- Add list skill function to web api - ([7f8e7c7](https://github.com/Aleph-Alpha/pharia-kernel/commit/7f8e7c76745d60b6d2c4b9496d1f4b1428bcb6da))
- Unload remove skills from cache - ([814748e](https://github.com/Aleph-Alpha/pharia-kernel/commit/814748eebbe72ce8632feb4abc5f7bfd61effde5))
- Shell wait for all config has been loaded once - ([69dd228](https://github.com/Aleph-Alpha/pharia-kernel/commit/69dd22864f89f06d55838b22bff0cdf834b71068))
- Improve error message for skill config fetch issues - ([9519a08](https://github.com/Aleph-Alpha/pharia-kernel/commit/9519a08f961e3ea82db983f904db0bb744cdf2f5))
- Avoid expecting operator config file for fallback - ([1b9c54b](https://github.com/Aleph-Alpha/pharia-kernel/commit/1b9c54b51791fe23dc928624eabcf91c87b98aff))
- Allow configuring operator config path via env var - ([4aa62b4](https://github.com/Aleph-Alpha/pharia-kernel/commit/4aa62b4457c77b3ab5489f50ab6eee318da291d1))
- Load file registry from config - ([147b41b](https://github.com/Aleph-Alpha/pharia-kernel/commit/147b41b311f7651693be5fffd846f301852f9f8d))
- Allow configuration file namespace - ([677d9ec](https://github.com/Aleph-Alpha/pharia-kernel/commit/677d9ec27414e07e51db0034767bd7aa7930e827))
- Support concurrent inference requests - ([e64061e](https://github.com/Aleph-Alpha/pharia-kernel/commit/e64061eeb0d6e6e274f6e67302f8fb5be74ff58e))
- Separate Observer Actor - ([a1c6cb1](https://github.com/Aleph-Alpha/pharia-kernel/commit/a1c6cb100b729549465bf787833f3481c0a94627))
- ConfigurationObserver tokio thread and shutdown with watchdog - ([1ee1c74](https://github.com/Aleph-Alpha/pharia-kernel/commit/1ee1c743a5e58f96f513cd6b3732dd34dd70e682))
- Default 60 seconds on bad string - ([c8193de](https://github.com/Aleph-Alpha/pharia-kernel/commit/c8193dee88dbfe6663a03cc1a589d278fd15b7ec))
- Introduce optional team_config token - ([0bcb2b7](https://github.com/Aleph-Alpha/pharia-kernel/commit/0bcb2b77fd781562da1f54ff84ce5a1fc0ac62d5))
- Support file url as skill config - ([be2174f](https://github.com/Aleph-Alpha/pharia-kernel/commit/be2174f631f2a5186285d7f6611e0a2333b02732))
- Add namespace configuration - ([821d411](https://github.com/Aleph-Alpha/pharia-kernel/commit/821d4111a38d717636da6243336069f37141d6bb))
- Skill config update interval configurable via env variable - ([178c3a0](https://github.com/Aleph-Alpha/pharia-kernel/commit/178c3a0d0b3b01c5b709ff656b9085aea9a35bb3))
- Load skill config before usage - ([6c4b816](https://github.com/Aleph-Alpha/pharia-kernel/commit/6c4b816a3555be997fb1e844df4d59e2d4cda5f3))
- Fetch skill config - ([34f9453](https://github.com/Aleph-Alpha/pharia-kernel/commit/34f94535bfccd1b0818384dce14bdf316555a559))
- Add gitlab skill config (draft) - ([79f7687](https://github.com/Aleph-Alpha/pharia-kernel/commit/79f7687caa27a39de69778b627bdccfcf459818a))
- Only allow specified skills if configuration file is provided - ([09a416e](https://github.com/Aleph-Alpha/pharia-kernel/commit/09a416ed0b6fc6cabee1371127d20174f04adcd9))
- Add skill config handling - ([f599ce9](https://github.com/Aleph-Alpha/pharia-kernel/commit/f599ce958662144ebb3d72c0dfacb78640c98612))
- Support v0.2.0 version of the WIT world - ([3781fc2](https://github.com/Aleph-Alpha/pharia-kernel/commit/3781fc2665a5d9bdbab4f80b85cbb49cccc2e0ad))
- Support stop sequences in inference client - ([eea68e8](https://github.com/Aleph-Alpha/pharia-kernel/commit/eea68e80567e3980c446e6e02a1789be14c259c8))
- Default to None for maximum tokens - ([bb7595c](https://github.com/Aleph-Alpha/pharia-kernel/commit/bb7595c0fe8dcd8663d591632dd784ced27b560f))
- Csi complete returns string only - ([2ce93ee](https://github.com/Aleph-Alpha/pharia-kernel/commit/2ce93ee6cdd7bca8aeee8a47415799fe8c52340e))
- Add ability to load and run v0.1 skills - ([906b369](https://github.com/Aleph-Alpha/pharia-kernel/commit/906b369a655e647bb0a8a46af48ce7d489ab5d1b))
- V0.1.0 based skill - ([b9a89f9](https://github.com/Aleph-Alpha/pharia-kernel/commit/b9a89f9a56f38d6ac98f500545a262081354ee45))
- Parse wit world on component loading - ([1df6503](https://github.com/Aleph-Alpha/pharia-kernel/commit/1df650355d8a7ebf2a7aadcba8a5c4eb43c3d45c))
- Serve skill wit statically - ([1b5a475](https://github.com/Aleph-Alpha/pharia-kernel/commit/1b5a475422d748f2c94408599acf6e5c47ba7fd8))
- Add pharia-skill run subcommand - ([4a2f3ec](https://github.com/Aleph-Alpha/pharia-kernel/commit/4a2f3ec62413483f0b798a9925850626e4f63bd3))
- Expose startup future that awaits bind operation - ([d52dc63](https://github.com/Aleph-Alpha/pharia-kernel/commit/d52dc6303741bd3f4b34475860807cf98d473e50))
- Add shell endpoint to drop skill from cache - ([d9850e1](https://github.com/Aleph-Alpha/pharia-kernel/commit/d9850e1ec6a04cd4450ad96a0ec868b5bdb0a597))
- Update validation error message - ([fed1fe0](https://github.com/Aleph-Alpha/pharia-kernel/commit/fed1fe0face934130aa72c9e93b4940f594c89da))
- Expose inference address to env - ([466e269](https://github.com/Aleph-Alpha/pharia-kernel/commit/466e269e47902bb1cece871ccab4cd8a06b5bf5c))
- Shell lists cached skills - ([a579176](https://github.com/Aleph-Alpha/pharia-kernel/commit/a5791762597878df4ca529d0953bac271bd808d4))
- Expose custom log env variable - ([d20db8c](https://github.com/Aleph-Alpha/pharia-kernel/commit/d20db8c91c6175d0b9298dde73cd36ec98ae9168))
- Add healthcheck endpoint - ([0a637ca](https://github.com/Aleph-Alpha/pharia-kernel/commit/0a637cac75211c999e463e1343f575b573204178))
- Add route to docs - ([44ffaee](https://github.com/Aleph-Alpha/pharia-kernel/commit/44ffaeee5226f27a4c1c14b8348577f6df74f83b))
- Use tracing instead of direct writing to stderr - ([3ffb2f0](https://github.com/Aleph-Alpha/pharia-kernel/commit/3ffb2f05413cc3e0aeb0b5727539381c6be92bb4))
- Capture wasm csi invocation failures - ([9935c15](https://github.com/Aleph-Alpha/pharia-kernel/commit/9935c15dfa35226f9c47083faaa762700569cf7f))
- Add go skill - ([6cd73ce](https://github.com/Aleph-Alpha/pharia-kernel/commit/6cd73ce0076eda21919fc798882ab737d3078bb6))
- Redirect root to docs - ([ffd02f7](https://github.com/Aleph-Alpha/pharia-kernel/commit/ffd02f7ed75161311f9fa9939a32707dc1e74e9d))
- Introduce static documentation sites - ([21cf3cf](https://github.com/Aleph-Alpha/pharia-kernel/commit/21cf3cff9807d90df2d218b1316b37d3b6cb75a5))
- Add publish skill sub-command - ([13d7a3c](https://github.com/Aleph-Alpha/pharia-kernel/commit/13d7a3c646aec1596042e07c6c7b5e841d12fa1c))
- Introduce pharia skill cli - ([24ed3b4](https://github.com/Aleph-Alpha/pharia-kernel/commit/24ed3b4ac26601e3019a0393a5c006a43098586c))
- Add oci runtime - ([4dfcaeb](https://github.com/Aleph-Alpha/pharia-kernel/commit/4dfcaeb420ae647c1f14b90e5b5083b12f6d0137))
- Impl remote wasm repository - ([0128d6f](https://github.com/Aleph-Alpha/pharia-kernel/commit/0128d6f7d51e8a5265398a986fe167d5202afa12))
- Implement retries for text completion - ([f69e0aa](https://github.com/Aleph-Alpha/pharia-kernel/commit/f69e0aa9be4119a1d27489956ebea95eb65fe196))
- Removed eager skill loading - ([9dd2cc2](https://github.com/Aleph-Alpha/pharia-kernel/commit/9dd2cc283d7a595a2d97110f65dcb186c3e92d36))
- Lazy loading skills by name - ([dfe6009](https://github.com/Aleph-Alpha/pharia-kernel/commit/dfe6009f1b3de75890f91d8cca8674d8bad2287a))
- Load from skills directory - ([2175e04](https://github.com/Aleph-Alpha/pharia-kernel/commit/2175e04cb90a54bb96daf1411b25fd2b71f418ba))
- Support invocation of arbitrary skills - ([47a0531](https://github.com/Aleph-Alpha/pharia-kernel/commit/47a05317d8fe52d22d76888121c1432cf18928dc))
- Use WasmRuntime - ([80cec78](https://github.com/Aleph-Alpha/pharia-kernel/commit/80cec78d8f3118a03e2ad3527dcbbe71fe691fdc))
- Read api_token from completion endpoint - ([ed5bd1c](https://github.com/Aleph-Alpha/pharia-kernel/commit/ed5bd1cfacd49acf4d80eb9263b9f63108046550))
- Add shutdown for inference - ([2edbe2f](https://github.com/Aleph-Alpha/pharia-kernel/commit/2edbe2ff4389d4191cabc7b93fa7fd48825d0a9b))
- Complete text added - ([740a753](https://github.com/Aleph-Alpha/pharia-kernel/commit/740a7533e2013173805a4e63e129680314d06a94))
- Add gracefull shutdown - ([866b2df](https://github.com/Aleph-Alpha/pharia-kernel/commit/866b2dfd2b0f7a7a99c24c9d65d94494a5d9c2d4))
- Configure host and port from environment - ([3b6a727](https://github.com/Aleph-Alpha/pharia-kernel/commit/3b6a7275753af2eb49e824009c2190a42d34a5c1))
- Hello world web service - ([ca65067](https://github.com/Aleph-Alpha/pharia-kernel/commit/ca650675844ebb58d9ab3986e4480fa8723b43d0))
- Hello world - ([0e9f936](https://github.com/Aleph-Alpha/pharia-kernel/commit/0e9f93686994806c1d2ea59509de01dc2b783259))

### Fixes

- Fit information in line for eval - ([6d3fc9d](https://github.com/Aleph-Alpha/pharia-kernel/commit/6d3fc9da9a0f7f34a8c0ac882262b349ebafda6f))
- Remove button underline - ([ba3de2d](https://github.com/Aleph-Alpha/pharia-kernel/commit/ba3de2db61ceac5821d8d3e8ab85c6c8fc049c22))
- Typo dot on newline in index page - ([03f28ab](https://github.com/Aleph-Alpha/pharia-kernel/commit/03f28ab3d0dbc8cc9c786b78cc4d3d7b195726a4))
- Spawn blocking thread to turn bytes into skill - ([cd676a3](https://github.com/Aleph-Alpha/pharia-kernel/commit/cd676a3d0365584d1f81de5e33ff33f46fe42c4e))
- Arrow direction in actor tam diagram - ([ffc98d2](https://github.com/Aleph-Alpha/pharia-kernel/commit/ffc98d2f818c09bd7ff6d3863c73c4338233377c))
- CONFIG_UPDATE_INTERVAL is now NAMESPACE_UPDATE_INTERVAL and it is parsed correctly - ([224e0ae](https://github.com/Aleph-Alpha/pharia-kernel/commit/224e0aeebd3064e9cf355a29b6d2eeadc2498282))
- Improve error message when skill is configured but not loadable - ([33669dd](https://github.com/Aleph-Alpha/pharia-kernel/commit/33669dd29c896bb139a50126081cd517cf68986e))
- Fallback to log level error if none is provided - ([ae861a5](https://github.com/Aleph-Alpha/pharia-kernel/commit/ae861a57dd615b3791e26b08c5ab1092bacfa3c0))
- Do not send remove message if only the skill tag changes - ([a7e22c8](https://github.com/Aleph-Alpha/pharia-kernel/commit/a7e22c8a8440b9790c921059e4ae55332cf60ea3))
- Lint unnecessary blank in markdown - ([31101b5](https://github.com/Aleph-Alpha/pharia-kernel/commit/31101b5db5590f9646bc698dba259c8fe3758c5b))
- Mark errors loading namespace description from file unrecoverable - ([29aea2f](https://github.com/Aleph-Alpha/pharia-kernel/commit/29aea2f4ae64337d1a635d75c32461997fc855aa))
- Typos - ([1a95629](https://github.com/Aleph-Alpha/pharia-kernel/commit/1a95629af006c09183ed175b3d1407e611671c8e))
- Lint, replace if panic with assert - ([f4cc965](https://github.com/Aleph-Alpha/pharia-kernel/commit/f4cc96586f21a4195d15a55404fb9273c1a8d3f2))
- Lint for missing Errors doc - ([c2ae251](https://github.com/Aleph-Alpha/pharia-kernel/commit/c2ae2514e6d7688206e8e3c86167d676cfbbb80d))
- Update mdbook docs copy destination - ([22f4836](https://github.com/Aleph-Alpha/pharia-kernel/commit/22f48365de167352bb5b2845279f8162e7fb5d43))
- Fix link in readme - ([ee09bac](https://github.com/Aleph-Alpha/pharia-kernel/commit/ee09bac33e93cb5ca5e1a50e7aef22458d4d4c1d))
- Inspect skills in namespace config - ([1dbc1ee](https://github.com/Aleph-Alpha/pharia-kernel/commit/1dbc1ee3c6496e76b3f704cde58a1f2354af3e68))
- Updates for finish reason - ([deb4395](https://github.com/Aleph-Alpha/pharia-kernel/commit/deb43953faaecfba66f9a57c5921377058a0a8df))
- Actually pass through stop reason in CSI - ([aa9fb32](https://github.com/Aleph-Alpha/pharia-kernel/commit/aa9fb32d07d6b8bff0fab402b3e0ae3bb9d0b69a))
- Limit max-tokens to 128 in previous wit world implementations - ([51a6386](https://github.com/Aleph-Alpha/pharia-kernel/commit/51a6386cf7bc25935dd6ab141aa991bb7ad569c5))
- Wit world with duplicate error - ([6581f7c](https://github.com/Aleph-Alpha/pharia-kernel/commit/6581f7c85baca8f5ac95324ba513551a31e4b7e2))
- Remove usage info - ([8647b79](https://github.com/Aleph-Alpha/pharia-kernel/commit/8647b794867a30c27f5ecfe0d2fb19bf0ce143eb))
- Api-docs for skill.wit route - ([12ddac8](https://github.com/Aleph-Alpha/pharia-kernel/commit/12ddac8490369ea64b9b00a401d68b95301c0ae6))
- Typo in cargo toml - ([af89e2f](https://github.com/Aleph-Alpha/pharia-kernel/commit/af89e2f0ed9b10cc498e84dcaf36ea2ce473b7c0))
- Internal server error on failing skill execution - ([f4657b4](https://github.com/Aleph-Alpha/pharia-kernel/commit/f4657b4bd2cd363ec4f66cfc94830c43c9f8285d))
- Do not allow execution of blank skill name - ([c84f452](https://github.com/Aleph-Alpha/pharia-kernel/commit/c84f4520c8eb050446631cea3b73e2075a07a45f))
- Format bind error context argument - ([d455606](https://github.com/Aleph-Alpha/pharia-kernel/commit/d455606fac80650ab15f9957e0940633b1449194))
- Bump max_tokens to 128 - ([0978920](https://github.com/Aleph-Alpha/pharia-kernel/commit/097892064491b4a5f22c6b1ea71b35afd379ec51))
- Load skill returns none if download failes - ([df600cb](https://github.com/Aleph-Alpha/pharia-kernel/commit/df600cbabd89f8b70b35da469d187b9246c66729))
- Try allowing venv in ci - ([805ba63](https://github.com/Aleph-Alpha/pharia-kernel/commit/805ba6380f90e7b2938869652a40cea68d1b8d86))
- Remove 20\*\* editions - ([081ba10](https://github.com/Aleph-Alpha/pharia-kernel/commit/081ba10818c4cfc6d127181560621a91abf038d7))
- Build script for new polyfill - ([61a7f10](https://github.com/Aleph-Alpha/pharia-kernel/commit/61a7f1037d4363487f34b79557ca0df2208ff31d))
- Run build skill script and copy skills folder - ([37106fc](https://github.com/Aleph-Alpha/pharia-kernel/commit/37106fc78c6ce0a0293019a88114ef1e7eda5be3))
- Bind to 0.0.0.0 instead of localhost to allow access from outside of docker - ([fa1e76d](https://github.com/Aleph-Alpha/pharia-kernel/commit/fa1e76d6a8d6547d4b709a5d93f120496697cd5e))
- Wrong dir name in containerfile - ([0a471c2](https://github.com/Aleph-Alpha/pharia-kernel/commit/0a471c2dc12ab63dd3d71f399a2275de031def6d))

### Documentation

- Tweak link - ([3020a73](https://github.com/Aleph-Alpha/pharia-kernel/commit/3020a730eda910e4d70e4441eb8af504654a701c))
- Fix links - ([52569bf](https://github.com/Aleph-Alpha/pharia-kernel/commit/52569bf29bd1aa576977992af30bd5c5192a3d31))
- Typo - ([ae13c39](https://github.com/Aleph-Alpha/pharia-kernel/commit/ae13c39f41e2d9230d502463bca3c83668521609))
- Fix MD lint issues and update feedback links - ([7944e8e](https://github.com/Aleph-Alpha/pharia-kernel/commit/7944e8e02d62dac56adc75227be8b1a1a0a33aa5))
- Update README with minor fixes - ([a47b828](https://github.com/Aleph-Alpha/pharia-kernel/commit/a47b828d3ff4721c970ca29b87ff4677699b8f45))
- Update Pooling Allocator Findings - ([21fd8a5](https://github.com/Aleph-Alpha/pharia-kernel/commit/21fd8a5bd572c6ad95095d878b22a7b665dd1775))
- Clarify Pharia Kernel shutdown behavior - ([adbe920](https://github.com/Aleph-Alpha/pharia-kernel/commit/adbe9207fd78b46e4c86b5c5aaf2a8f32fc80fff))
- Update resource evaluation findings - ([5943f9e](https://github.com/Aleph-Alpha/pharia-kernel/commit/5943f9ec492aec21834c37e0ae923425a99e531b))
- Update FINDINGS.md - ([586a194](https://github.com/Aleph-Alpha/pharia-kernel/commit/586a194e171fc82cd269e5816ad2ed12ebe0b684))
- Update resource evaluation documentation - ([245251f](https://github.com/Aleph-Alpha/pharia-kernel/commit/245251ff94f40aab0dccdebc13b446ac8954f25b))
- Add AA_INFERENCE_ADDRESS as it is now required for running tests - ([762ca6e](https://github.com/Aleph-Alpha/pharia-kernel/commit/762ca6ee3a7638b938c800658039ab4ffa9d01e1))
- Fix formatting report eval - ([ceecb5a](https://github.com/Aleph-Alpha/pharia-kernel/commit/ceecb5a7ff25e936f6b43d39043705b885eb1a1e))
- Update resource evaluation report formatting - ([87e8111](https://github.com/Aleph-Alpha/pharia-kernel/commit/87e8111c618de8322bdce95925385fd85dfaa402))
- Add log file evaluation - ([9dc98f5](https://github.com/Aleph-Alpha/pharia-kernel/commit/9dc98f50325b87878054d933ac8eca411d4e3189))
- Update resource_eval README documentation - ([be90b94](https://github.com/Aleph-Alpha/pharia-kernel/commit/be90b94a4efbc7cee76ea73152f4374475233fd1))
- Add resource evaluation tool, start findings - ([ec608a2](https://github.com/Aleph-Alpha/pharia-kernel/commit/ec608a2f7f1c5e6e5d100d6dc7b5c736a392dc71))
- Add testing section for Python SDK - ([9526041](https://github.com/Aleph-Alpha/pharia-kernel/commit/9526041f1c5844913ef97a72bf3bcd2e37225059))
- Fix arrow direction - ([603a2be](https://github.com/Aleph-Alpha/pharia-kernel/commit/603a2be0ef11ac4d0a8d807164257223467bc62a))
- Fix typo in actor block diagram - ([e9bdeb4](https://github.com/Aleph-Alpha/pharia-kernel/commit/e9bdeb45e947de4314b1a925b9817ba75cfff5f9))
- Add csi component - ([9c1f892](https://github.com/Aleph-Alpha/pharia-kernel/commit/9c1f892b5fbbd91e0b9fcce4ee7f42eea7fed9e8))
- Dedicate separate page for deploying skills - ([601bbfa](https://github.com/Aleph-Alpha/pharia-kernel/commit/601bbfab55becd3381d716d24aec690c9a7b169a))
- Python SDK - ([2e606b5](https://github.com/Aleph-Alpha/pharia-kernel/commit/2e606b5cb0e58d87700ce88fe191c17adcc2cf2e))
- Fix typo - ([eaa4400](https://github.com/Aleph-Alpha/pharia-kernel/commit/eaa44003c0af344394ef8bf815f483859425a2b4))
- Dedicate separate page for local Pharia Kernel setup - ([cc68b71](https://github.com/Aleph-Alpha/pharia-kernel/commit/cc68b71ef87dc01c367cd4ffa4efe942ae1da644))
- Markdownlint + remove outdated section - ([6ac0fff](https://github.com/Aleph-Alpha/pharia-kernel/commit/6ac0fff9487dd4782b295f4cdb5b92096e0ceee4))
- Update motivation in readme - ([dfcd464](https://github.com/Aleph-Alpha/pharia-kernel/commit/dfcd464b502b4b780455fe715db55bd54c534446))
- Update skill development - ([172fc75](https://github.com/Aleph-Alpha/pharia-kernel/commit/172fc75c4d1ef6d3f7dcc8beffb727ac3c585124))
- Add test dependencies to deployment block diagram - ([56d4587](https://github.com/Aleph-Alpha/pharia-kernel/commit/56d4587c2a41f58de7be6b00fbe1315011413f75))
- Deployment SVG - ([40b9e2b](https://github.com/Aleph-Alpha/pharia-kernel/commit/40b9e2b6160edd49a118ada5f513f022b1e8a72d))
- Context diagram to tell integration story - ([a8739e8](https://github.com/Aleph-Alpha/pharia-kernel/commit/a8739e8279d7f8b6d360b5c52fe99204633a7c2b))
- Block diagram for actors - ([4825377](https://github.com/Aleph-Alpha/pharia-kernel/commit/48253777f1b99f137d3fb04951e12d55fc47009b))
- Fix namespace update interval value - ([6ed8c81](https://github.com/Aleph-Alpha/pharia-kernel/commit/6ed8c81aa0d428297bc99bd98ab7d20aa563bfed))
- Fix cache invalidation curl call - ([739e1a4](https://github.com/Aleph-Alpha/pharia-kernel/commit/739e1a44490e48ca210621fbe7922ae682637e64))
- Improve local skill development flow - ([18d7392](https://github.com/Aleph-Alpha/pharia-kernel/commit/18d7392f5f38bdc1a57ebaf74442a10a012b8045))
- Fix typos - ([9dbad56](https://github.com/Aleph-Alpha/pharia-kernel/commit/9dbad560b980095fb161725eba94f8e934f4a10b))
- Document local skill development setup - ([b83d462](https://github.com/Aleph-Alpha/pharia-kernel/commit/b83d462c6f6ea6213a3aeee660e7ec9a529dcfb6))
- Add observability - ([1dd619e](https://github.com/Aleph-Alpha/pharia-kernel/commit/1dd619efa37834eaa8c8b80e1da0cb02ecd74766))
- Update skill python code example - ([cf7d391](https://github.com/Aleph-Alpha/pharia-kernel/commit/cf7d391131e24d624aeaf65046ca47e63ec79f12))
- Updated skill developer documentation to use the newest wit world - ([1af30df](https://github.com/Aleph-Alpha/pharia-kernel/commit/1af30df7383805e3dd189a4385a7fafeeea7de4b))
- Fix execute_skill examples, use JSON - ([03d8285](https://github.com/Aleph-Alpha/pharia-kernel/commit/03d8285a6ad5751521f88b91cafefeea79639e6f))
- Add info regarding requirements for AA_API_TOKEN - ([c02104f](https://github.com/Aleph-Alpha/pharia-kernel/commit/c02104f8f8bd2aefa9731dfaa3db4d645b01aeb5))
- Readme, cleanup podman instructions - ([74bb018](https://github.com/Aleph-Alpha/pharia-kernel/commit/74bb01894b12c5bbbfa95542b4efcfa8fc84eceb))
- Mention PHARIA_KERNEL_ADDRESS expected default for running container - ([87fd6df](https://github.com/Aleph-Alpha/pharia-kernel/commit/87fd6dfbc716b18e2c4351db80f41d198e315d02))
- Update build for mac - ([a1bd0ae](https://github.com/Aleph-Alpha/pharia-kernel/commit/a1bd0aeb188fddba596b267949a7b14883de8ffd))
- Specify podman requirements in README - ([7b07073](https://github.com/Aleph-Alpha/pharia-kernel/commit/7b07073c4952fa858756b949793c3acbf534243a))
- Explain await.await syntax - ([8640b61](https://github.com/Aleph-Alpha/pharia-kernel/commit/8640b61be37243f9aadfc1cbb8f6c28250af97e3))
- Add some blank lines - ([d220777](https://github.com/Aleph-Alpha/pharia-kernel/commit/d220777ed8ec98a0efe752b8c59c232d0dbf682e))
- Add curl examples - ([9812169](https://github.com/Aleph-Alpha/pharia-kernel/commit/9812169e94f225b3d0ca2887e224fbb2f3f33b4d))
- Add volumen and env file to podman run command - ([6a37653](https://github.com/Aleph-Alpha/pharia-kernel/commit/6a3765362bea1838dd5850f4a754ceb7a67a91ed))
- Explain namespace concept better and mention how to authorize repository access - ([011599c](https://github.com/Aleph-Alpha/pharia-kernel/commit/011599c5a3b6e55b5ac02d383654fbbee2e20657))
- Introduce namespace and tags - ([37f144e](https://github.com/Aleph-Alpha/pharia-kernel/commit/37f144e2287ab464e7c80b379606a4cc1ce1e28e))
- Add skills path to API docs - ([28c944e](https://github.com/Aleph-Alpha/pharia-kernel/commit/28c944e9a1164ebc2e98aaea26e3260634fc213b))
- Adjusted execute skill api doc - ([2431b0d](https://github.com/Aleph-Alpha/pharia-kernel/commit/2431b0d92884db092b009ea6fbf9bec75d32b1cc))
- Doc comments for engine and linker responsibilities - ([61ec10d](https://github.com/Aleph-Alpha/pharia-kernel/commit/61ec10db06391680de065e930ae455c1bae58b7c))
- Skill_wit - ([4df0f7e](https://github.com/Aleph-Alpha/pharia-kernel/commit/4df0f7e5150c5d7d13fb1579e54ebd9223688a7e))
- Mention access rights on registry tokens - ([2a67c8c](https://github.com/Aleph-Alpha/pharia-kernel/commit/2a67c8cad0003c9d9c1464c5dd9351761bb82001))
- Document cached skill delete endpoint - ([67cf9a0](https://github.com/Aleph-Alpha/pharia-kernel/commit/67cf9a020aba10b1ed8b60df1847cee3dbca2eeb))
- Help users to find the user manual first - ([024798c](https://github.com/Aleph-Alpha/pharia-kernel/commit/024798cb641df3380b9319551479cabd12612737))
- More comments - ([a2a6612](https://github.com/Aleph-Alpha/pharia-kernel/commit/a2a66121b4d83833fe0d93d2d3085b0168d563aa))
- Extra comments - ([911e285](https://github.com/Aleph-Alpha/pharia-kernel/commit/911e28530f096539ca41cfd6b40a99d2d5c174ee))
- Update Skill repository - ([ab762a0](https://github.com/Aleph-Alpha/pharia-kernel/commit/ab762a0290f29b8dd987635d5a15c18ce1d66bf1))
- How to get access to Pharia Skill - ([4ea6311](https://github.com/Aleph-Alpha/pharia-kernel/commit/4ea6311440a76eb378fe813687aa16504b92d270))
- Cleanup docs and comments - ([bf86d3c](https://github.com/Aleph-Alpha/pharia-kernel/commit/bf86d3cc1a6d39a57bf5b145abc1ea21cce7f2ca))
- Change to utoipa instead of aide - ([dd11ea4](https://github.com/Aleph-Alpha/pharia-kernel/commit/dd11ea4a4a6d419f1d3047369f7586263835e3d7))
- Fix typos and style - ([d746a0e](https://github.com/Aleph-Alpha/pharia-kernel/commit/d746a0e18ac00bd64213c5066a9b5f943cb2edc0))
- Improved mdbook - ([941f1d2](https://github.com/Aleph-Alpha/pharia-kernel/commit/941f1d290895a02e53f220226e7aeb52c9052c6e))
- Add information on setting the log level - ([8793f07](https://github.com/Aleph-Alpha/pharia-kernel/commit/8793f07b1233ad5d7059f5c85fd2007273857038))
- Adjust mdbook structure - ([1569237](https://github.com/Aleph-Alpha/pharia-kernel/commit/1569237626ff5c5ad78b5e1296af1ba21a7c8d33))
- Add skill content - ([f638147](https://github.com/Aleph-Alpha/pharia-kernel/commit/f638147a60729ce9e6cbf64415b99bc3f71222aa))
- Add skills content - ([f45ef52](https://github.com/Aleph-Alpha/pharia-kernel/commit/f45ef52de122b535aa4bcb1420f5eddd88cc12c2))
- Add mdbook structure - ([0f526e6](https://github.com/Aleph-Alpha/pharia-kernel/commit/0f526e6b6071ae39dcc60c7984dc9985fed369df))
- TAM Block deploying skills - ([597e294](https://github.com/Aleph-Alpha/pharia-kernel/commit/597e2946fd3ee11e929cf2b13eb383fb50a5fc2c))
- Add operations manual - ([83faab5](https://github.com/Aleph-Alpha/pharia-kernel/commit/83faab5a0e927d2ed5d6b58bd601ec6240826273))
- Remove the routes for docs - ([5b97813](https://github.com/Aleph-Alpha/pharia-kernel/commit/5b97813a355b4f6631bd74d038458fe3ae7c747e))
- Add user manual readme section - ([7f6ef68](https://github.com/Aleph-Alpha/pharia-kernel/commit/7f6ef68b05ac2727f834e5dfdfb17c1dacf958fd))
- Introduce mdbook - ([81080b9](https://github.com/Aleph-Alpha/pharia-kernel/commit/81080b9a61b4ddb46fd181fc07a1fce1791a573f))
- Add link to status page - ([a9e2460](https://github.com/Aleph-Alpha/pharia-kernel/commit/a9e246065bc79114bc6a70e411b7eecb4dbb1dad))
- Move container steps to the Contributing section - ([80e5e9e](https://github.com/Aleph-Alpha/pharia-kernel/commit/80e5e9e08ebff7b27823a1322631268d81e59498))
- Reorder readme - ([d59317b](https://github.com/Aleph-Alpha/pharia-kernel/commit/d59317bd9a021a99fb0cec3790e00b4d4c5b49e3))
- Explain signature of run_greet - ([2d26e33](https://github.com/Aleph-Alpha/pharia-kernel/commit/2d26e334ff68d38c4be32b2087e64e9c67c611ff))
- Add wasm build commands - ([75fa1b0](https://github.com/Aleph-Alpha/pharia-kernel/commit/75fa1b0ade886e272e63644cf29aeacc34bfb36b))
- Entity relationship skills - ([9641829](https://github.com/Aleph-Alpha/pharia-kernel/commit/9641829474839eb0a03ff2e44a8f5ddd9614ae9c))
- Fix typo in README - ([b7d723c](https://github.com/Aleph-Alpha/pharia-kernel/commit/b7d723c8bc1a119396a379c3555ba1b5aad12832))
- Specify container name in README - ([a8bb41a](https://github.com/Aleph-Alpha/pharia-kernel/commit/a8bb41a26b41e4129ab1d1eb8e04435917880213))
- Docker-env variable for building on Apple Silicon - ([e17f9bd](https://github.com/Aleph-Alpha/pharia-kernel/commit/e17f9bd474c511d2fdad031dc883ad416eed155d))
- Add developer section on how to build and run the docker image - ([ca58929](https://github.com/Aleph-Alpha/pharia-kernel/commit/ca58929d042764625ae36eaeede1c08bcd04b2ef))
- Overview on premise deployments - ([f8bdf67](https://github.com/Aleph-Alpha/pharia-kernel/commit/f8bdf6797022d13d388065f1c1f468b6576d0f3d))
- Block Diagram running Pharia OS - ([94ed860](https://github.com/Aleph-Alpha/pharia-kernel/commit/94ed860d15e2a136c1216f7ed43856507b175d95))
- Block diagrom kernel overview - ([6e41c80](https://github.com/Aleph-Alpha/pharia-kernel/commit/6e41c809d69290c802ad9089316dedf0f4c47543))
- Readme stating conv commits - ([b0cce9a](https://github.com/Aleph-Alpha/pharia-kernel/commit/b0cce9a578edde569ad0e0262cab8fad6050ffde))

### Performance

- Add memory chunking to resource evaluation samples - ([46acbed](https://github.com/Aleph-Alpha/pharia-kernel/commit/46acbed5624e1204b283daa894853c8b4549ddb5))
- Update resource evaluation findings and commands - ([cc9fd47](https://github.com/Aleph-Alpha/pharia-kernel/commit/cc9fd479f080c06216a49a2498819a8fcd02cc65))
- Add parallel execution commands - ([cea3896](https://github.com/Aleph-Alpha/pharia-kernel/commit/cea3896938eab1c81f10fb1daabe4fedcff38076))
- Avoid unnecessary cloning - ([3dea844](https://github.com/Aleph-Alpha/pharia-kernel/commit/3dea844c7117fe32564e537e987138703b060463))
- Remove unnecessary cloning - ([2e468ae](https://github.com/Aleph-Alpha/pharia-kernel/commit/2e468aedf1a33960fd617c9664980762350e51fd))

### Builds

- Exclude resource_eval from the Pharia Kernel package - ([be1112a](https://github.com/Aleph-Alpha/pharia-kernel/commit/be1112a8e367268da54b4aef340565d7b5b7e808))
- Exclude Helm charts from the Pharia Kernel package - ([8eff879](https://github.com/Aleph-Alpha/pharia-kernel/commit/8eff879673424a77a4df25598ba5ce383f27e6b0))
- Warn only on linkcheck issues - ([df3acbf](https://github.com/Aleph-Alpha/pharia-kernel/commit/df3acbf3c7f5374eae61402ae72d1974dafbdb7d))
- Strip skill files - ([3d43641](https://github.com/Aleph-Alpha/pharia-kernel/commit/3d4364170fc682fb75d864e559bcc4ffa82e7e12))
- Update container working directory - ([8a7fa7f](https://github.com/Aleph-Alpha/pharia-kernel/commit/8a7fa7fdfda3609aff8024c067110ef60967145b))
- Specify work dir - ([45c2784](https://github.com/Aleph-Alpha/pharia-kernel/commit/45c278474b97f5f9e95b9028fb8cedfca476acc9))
- Add build script for go greet skill - ([e1a7258](https://github.com/Aleph-Alpha/pharia-kernel/commit/e1a72586b9ece7d875d5657cf078036a3dd6f53f))
- Add wasm32-wasi target in container file - ([7fce15c](https://github.com/Aleph-Alpha/pharia-kernel/commit/7fce15cc5c0007957b9d44620cf0cb5e2cf24485))
