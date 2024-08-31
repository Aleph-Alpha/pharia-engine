# Pharia Kernel Progress Update

* Introduction, set the scene
* Listing problems **we** have rolling stuff out in Production, Deploying
  * Provide Secure (Container) Images
  * Wire it up to uptime Monitoring
  * Argo CD / Helmcharts
  * Skill code does not need to care about tokens (we forward permissions for them, for now)
  * No handling of Runtime errors required in Skill code
  * Automatic handling of Busy errors
* Dynamically add/remove Skills securely without redeploying Kernel
* Securely run user defined code
* Live demo of Haiku Python example
* Mention Go- and Rust on the Slides

## Caveats

* Currently we only execute one skill at a time
* One inference request at the time
* Anyone can deploy skills
* No namespaces for skills
* CSI is very limited
* ...

## Vision

* AI can write and deploy its own code in seconds. And it is sufficiently sandboxed to be trustworthy.
