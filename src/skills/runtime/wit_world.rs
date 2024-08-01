pub mod unversioned {
    use wasmtime::component::bindgen;

    bindgen!({ world: "skill", path: "./wit/skill@unversioned", async: true });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use semver::Version;
    use wit_parser::decoding::{decode, DecodedWasm};

    fn extract_pharia_skill_version(wasm: &[u8]) -> anyhow::Result<Option<Version>> {
        let decoded = decode(wasm)?;
        if let DecodedWasm::Component(resolve, ..) = decoded {
            Ok(resolve
                .package_names
                .keys()
                .find(|k| (k.namespace == "pharia" && k.name == "skill"))
                .and_then(|k| k.version.clone()))
        } else {
            Ok(None)
        }
    }

    #[test]
    fn can_parse_module() {
        let wasm = fs::read("skills/greet_skill.wasm").unwrap();
        let version = extract_pharia_skill_version(&wasm).unwrap();
        assert_eq!(version, None);
    }
}
