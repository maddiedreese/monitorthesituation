use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG: &str = r#"# monitorthesituation
# Add streams you are authorized to view. Credentials can reference environment
# variables with ${NAME}; they are expanded at runtime and never printed.
version: 1

ui:
  renderer: blocks       # blocks | ascii
  color: true
  fps: 10
  columns: auto          # auto, or a number such as 2
  show_help: true
  ascii_ramp: " .:-=+*#%@"

sources:
  # - name: Harbor
  #   input: https://camera.example/live/index.m3u8
  #   kind: stream
  #
  # - name: Desk camera
  #   input: camera://0
  #   kind: camera
  []
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub ui: UiConfig,
    pub sources: Vec<SourceConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            ui: UiConfig::default(),
            sources: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub renderer: Renderer,
    pub color: bool,
    pub fps: u8,
    pub columns: Columns,
    pub show_help: bool,
    pub ascii_ramp: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            renderer: Renderer::Blocks,
            color: true,
            fps: 10,
            columns: Columns::Auto,
            show_help: true,
            ascii_ramp: " .:-=+*#%@".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    Ascii,
    Blocks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Columns {
    Auto,
    Fixed(u16),
}

impl Serialize for Columns {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::Fixed(n) => serializer.serialize_u16(*n),
        }
    }
}

impl<'de> Deserialize<'de> for Columns {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Text(String),
            Number(u16),
        }
        match Value::deserialize(deserializer)? {
            Value::Text(value) if value.eq_ignore_ascii_case("auto") => Ok(Self::Auto),
            Value::Text(value) => Err(serde::de::Error::custom(format!(
                "expected 'auto' or a positive number, got {value:?}"
            ))),
            Value::Number(0) => Err(serde::de::Error::custom("columns must be at least 1")),
            Value::Number(n) => Ok(Self::Fixed(n)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub name: String,
    pub input: String,
    #[serde(default)]
    pub kind: SourceKind,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

impl SourceConfig {
    pub fn display_input(&self) -> String {
        sanitized_input(&self.input)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    #[default]
    Auto,
    Stream,
    Camera,
    File,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("could not read configuration at {}", path.display()))?;
        let mut config: Self = serde_yaml_ng::from_str(&contents)
            .with_context(|| format!("invalid configuration at {}", path.display()))?;
        config.expand_environment()?;
        config.validate()?;
        Ok(config)
    }

    fn expand_environment(&mut self) -> Result<()> {
        for source in &mut self.sources {
            source.input = expand_environment(&source.input)?;
            for value in source.headers.values_mut() {
                *value = expand_environment(value)?;
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!(
                "unsupported configuration version {}; expected 1",
                self.version
            );
        }
        if !(1..=30).contains(&self.ui.fps) {
            bail!("ui.fps must be between 1 and 30");
        }
        if self.ui.ascii_ramp.chars().count() < 2 {
            bail!("ui.ascii_ramp needs at least two characters");
        }
        for (index, source) in self.sources.iter().enumerate() {
            if source.name.trim().is_empty() {
                bail!("sources[{index}].name cannot be empty");
            }
            if source.name.chars().any(char::is_control) {
                bail!("sources[{index}].name cannot contain control characters");
            }
            if source.input.trim().is_empty() {
                bail!("sources[{index}].input cannot be empty");
            }
            if source.input.chars().any(char::is_control) {
                bail!("sources[{index}].input cannot contain control characters");
            }
            for key in source.headers.keys() {
                if key.contains(['\r', '\n']) {
                    bail!("sources[{index}] contains an invalid header name");
                }
            }
            for value in source.headers.values() {
                if value.contains(['\r', '\n']) {
                    bail!("sources[{index}] contains an invalid header value");
                }
            }
        }
        Ok(())
    }
}

pub fn expand_environment(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            bail!("unterminated environment variable reference");
        };
        let name = &after[..end];
        if name.is_empty() || !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            bail!("invalid environment variable name {name:?}");
        }
        let value = std::env::var(name)
            .with_context(|| format!("environment variable {name} is not set"))?;
        output.push_str(&value);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

pub fn source_name(input: &str, index: usize) -> String {
    if let Some(device) = input.strip_prefix("camera://") {
        return format!("Camera {device}");
    }
    if let Some((_, rest)) = input.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or(authority);
        if !host.is_empty() {
            return host.to_owned();
        }
    }
    Path::new(input)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Situation {}", index + 1))
}

pub fn sanitized_input(input: &str) -> String {
    if input.starts_with("camera://") {
        return input.to_owned();
    }
    if let Some((scheme, rest)) = input.split_once("://") {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = authority.rsplit('@').next().unwrap_or(authority);
        return if host.is_empty() {
            format!("{scheme}://…")
        } else {
            format!("{scheme}://{host}/…")
        };
    }
    Path::new(input)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("local file")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_document_parses() {
        let config: Config = serde_yaml_ng::from_str(DEFAULT_CONFIG).unwrap();
        config.validate().unwrap();
        assert!(config.sources.is_empty());
    }

    #[test]
    fn columns_accept_auto_and_number() {
        let auto: Columns = serde_yaml_ng::from_str("auto").unwrap();
        let fixed: Columns = serde_yaml_ng::from_str("3").unwrap();
        assert_eq!(auto, Columns::Auto);
        assert_eq!(fixed, Columns::Fixed(3));
    }

    #[test]
    fn invalid_fps_is_rejected() {
        let mut config = Config::default();
        config.ui.fps = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn leaves_plain_values_unchanged() {
        assert_eq!(
            expand_environment("https://example.com/a#b").unwrap(),
            "https://example.com/a#b"
        );
    }

    #[test]
    fn source_display_hides_credentials_and_query_tokens() {
        let input = "https://person:password@camera.example/live.m3u8?token=secret";
        assert_eq!(sanitized_input(input), "https://camera.example/…");
        assert_eq!(source_name(input, 0), "camera.example");
    }
}
