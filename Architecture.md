# Architecture of Pharia Kernel

## System Structure

### Pharia Kernel - Pharia OS

![Pharia OS](./tam/pharia-os-running.drawio.svg)

### Pharia OS - Pharia AI

- Pharia Studio: Developer Tooling, Resource Management, Fine Tuning
- Pharia OS: Platform Services operated for Studio

## Software Structure

### Hexagonal Architecture

```rust
pub async fn run(app_config: AppConfig, shutdown_signal: impl Future<Output = ()> + Send + 'static) {
    // Boot up the drivers which power the CSI. Right now we only have inference.
    let inference = Inference::new(app_config.inference_addr);

    // Boot up runtime we need to execute Skills
    let skill_executor = SkillExecutor::new(inference.api());
    let skill_executor_api = skill_executor.api();

    // Make skills available via http interface. If we get the signal for shutdown the future
    // will complete.
    if let Err(e) = shell::run(app_config.tcp_addr, skill_executor_api, shutdown_signal).await {
        // We do **not** want to bubble up an error during shell initialization or execution. We
        // want to shutdown the other actors before finishing this function.
        error!("Could not boot shell: {e}");
    }

    // Shutdown everything we started. We reverse the order for the shutdown so all the required
    // actors are still answering for each component.
    skill_executor.wait_for_shutdown().await;
    inference.wait_for_shutdown().await;
}
```

### Actors

```rust
/// Handle to the inference actor. Spin this up in order to use the inference API.
pub struct Inference {
    send: mpsc::Sender<InferenceMessage>,
    handle: JoinHandle<()>,
}
```

```rust
/// Use this to execute tasks with the inference API. The existence of this API handle implies the
/// actor is alive and running. This means this handle must be disposed of, before the inference
/// actor can shut down.
#[derive(Clone)]
pub struct InferenceApi {
    send: mpsc::Sender<InferenceMessage>,
}
```

```rust
/// Private implementation of the inference actor running in its own dedicated green thread.
struct InferenceActor<C> {
    // ...other members ...
    recv: mpsc::Receiver<InferenceMessage>,
}
```

--- Bonus: Handling of Runtime errors ---

```rust
async fn run_skill(
    &mut self,
    skill: String,
    input: String,
    api_token: String,
) -> anyhow::Result<String> {
    let (send_rt_err, recv_rt_err) = oneshot::channel();
    let ctx = Box::new(SkillInvocationCtx::new(
        send_rt_err,
        self.inference_api.clone(),
        api_token,
    ));

    select! {
        result = self.runtime.run(&skill, input, ctx) => result,
        Ok(error) = recv_rt_err => Err(error)
    }
}
```

```rust
async fn run_skill(
    &mut self,
    skill: String,
    input: String,
    api_token: String,
) -> anyhow::Result<String> {
    let (send_rt_err, recv_rt_err) = oneshot::channel();
    let ctx = Box::new(SkillInvocationCtx::new(
        send_rt_err,
        self.inference_api.clone(),
        api_token,
    ));
    select! {
        result = self.runtime.run(&skill, input, ctx) => result,
        Ok(error) = recv_rt_err => Err(error)
    }
}
```

```rust
#[async_trait]
impl Csi for SkillInvocationCtx {
    async fn complete_text(&mut self, params: CompleteTextParameters) -> String {
        match self
            .inference_api
            .complete_text(params, self.api_token.clone())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.send_rt_err
                    .take()
                    .expect("Only one error must be send during skill invocation")
                    .send(error)
                    .unwrap();
                // Never return, we did report the error via the send error channel.
                pending().await
            }
        }
    }
}
```
