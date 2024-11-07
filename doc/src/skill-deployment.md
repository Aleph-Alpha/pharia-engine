# Deploying Skills to the Kernel

In order to make a Skill available in Pharia Kernel two criteria need to be met:

* The skill must be deployed as a component to an OCI registry (a directory might also suffice for local development)
* The skill must be configured in the namespace configuration (a `toml` file, typically checked into a Git repository)

If your team does not own a namespace yet, you can request one from your Pharia Kernel operators. They will associate it with a `namespace.toml`, which lists the skills you want to deploy, as well as an OCI registry to load the skill code into. Ideally, your team owns both, so you can deploy skills in self service.

Skills are not containers. Yet, we still publish them into container repositories. Ask your administrator which ones are linked to your instance of Pharia Kernel. At Aleph Alpha we are currently using the registry `registry.gitlab.aleph-alpha.de` and the repository `engineering/pharia-skills/skills`. It is recommended to publish the Skill using the `pharia-skill` command line tool. `pharia-skill` is deployed as a container image to our Artifactory. You can acquire it with Podman like this.

**Attention:** We are currently still in a closed beta. Which means people outside of Aleph Alpha can not download our Pharia Kernel images. You may need to request a JFrog account, or extend the permission of your JFrog account to see images internal to the Aleph Alpha Organization. You can do so, by reaching out to us via the Product Service Desk:

* Service Desk: <https://aleph-alpha.atlassian.net/servicedesk/customer/portals>
* How to create a ticket: <https://aleph-alpha.atlassian.net/wiki/spaces/EN/pages/847937592/Aleph+Alpha+Service+Desk+-+How+To>

```shell
curl -L https://alephalpha.jfrog.io/artifactory/pharia-kernel-files/pharia-skill-cli/0.1.16/aarch64-apple-darwin -H "Authorization: Bearer $JFROG_TOKEN" -o pharia-skill-cli
chmod +x pharia-skill-cli
```

On Linux, use `x86_64-unknown-linux-gnu` instead of `aarch64-apple-darwin`.
On Windows, use `x86_64-pc-windows-msvc` and `chmod` is not necessary.

With the tooling available, we can now upload the Skill.

```shell
pharia-skill-cli publish -R registry.gitlab.aleph-alpha.de -r engineering/pharia-skills/skills -u DUMMY_USER_NAME -p $GITLAB_TOKEN ./haiku.wasm
```

With our GitLab registry, any user name will work, as long as you use a access token. You can generate a token on your profile page. It is necessary to include write privilege.

Everyone is welcomed to explore in the shared `playground` namespace, a skill registry is provided at <https://gitlab.aleph-alpha.de/engineering/pharia-kernel-playground>.

## Configuring namespace

As mentioned above, a namespace is associated with a `namespace.toml`. We have to configure the `namespace.toml` for the deployed skills to be loaded in Pharia Kernel.

The TOML file lists all skills in the namespace. Here is an example:

```toml
skills = [
    { name = "greet" },
    { name = "haiku", tag = "v1.0.1" },
]
```

With this configuration, Pharia Kernel looks in the associated OCI registry for two skills: the "greet" skill with the tag "latest" and the "haiku" skill with the tag "v1.0.1". If the configuration changes, Pharia Kernel will pick up the changes and reload the skills.

Congratulations! Your Skill is now deployed. You can now use the [HTTP API](https://pharia-kernel.aleph-alpha.stackit.run/api-docs#tag/skills/POST/execute_skill) to call it.
