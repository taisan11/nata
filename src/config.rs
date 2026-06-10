pub struct Config {
    pub treesitter: bool
}

impl nojson::DisplayJson for Config {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("treesitter", &self.treesitter)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Config {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let treesitter = value.to_member("treesitter")?.required()?;
        Ok(Config {
            treesitter: treesitter.try_into()?,
        })
    }
}

pub fn load_config(path: Option<String>) -> Result<Config, Box<dyn std::error::Error>> {
    match path {
        Some(path) => {
            let config_str = std::fs::read_to_string(path)?;
            let config: nojson::Json<Config> = config_str.parse()?;
            Ok(config.0)
        }
        None => Ok(Config { treesitter: true }),
    }
}
