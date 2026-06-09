pub struct Config {
    pub treesitter: bool
}

pub fn load_config() -> Config {
    Config {
        treesitter: true
    }
}
