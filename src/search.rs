mod actor;
mod client;

pub use actor::{Search, SearchApi};

#[cfg(test)]
pub mod tests {
    pub use super::actor::SearchMessage;
}
