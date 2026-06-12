pub struct Config {
    pub treesitter: bool,
    pub key_modifier: String,
}

impl nojson::DisplayJson for Config {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("treesitter", &self.treesitter)?;
            f.member("key_modifier", &self.key_modifier)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for Config {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        let treesitter = value.to_member("treesitter")?.required()?;
        let key_modifier = match value.to_member("key_modifier")?.optional() {
            Some(v) => {
                let s: String = v.try_into()?;
                s
            }
            None => "ctrl".to_string(),
        };
        Ok(Config {
            treesitter: treesitter.try_into()?,
            key_modifier,
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
        None => Ok(Config { treesitter: true, key_modifier: "ctrl".to_string() }),
    }
}
