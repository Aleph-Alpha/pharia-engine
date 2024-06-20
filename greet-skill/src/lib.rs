wit_bindgen::generate!({world: "skill"});

struct Skill {}

impl Guest for Skill {
    fn run(name: String) -> String {
        format!("Hello, {name}")
    }
}

export!(Skill);

#[cfg(test)]
mod tests {
    // use super::*;
}
