use serde::{Deserialize, Serialize};

/// Parsed representation of an app's `swallow.yaml`.
///
/// Only `name`, `domain` and at least one port are strictly required; everything
/// else has a sensible default so small apps can ship a tiny manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,

    #[serde(default = "default_dockerfile")]
    pub dockerfile: String,

    pub domain: String,

    #[serde(default)]
    pub ports: Vec<PortSpec>,

    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,

    #[serde(default)]
    pub resources: Resources,

    #[serde(default)]
    pub healthcheck: Option<Healthcheck>,

    #[serde(default)]
    pub scale: Scale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSpec {
    pub container: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resources {
    /// CPU limit in cores, e.g. "0.5". Empty = unlimited.
    #[serde(default)]
    pub cpu: Option<String>,
    /// Memory limit, e.g. "256m". Empty = unlimited.
    #[serde(default)]
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Healthcheck {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scale {
    #[serde(default = "default_instances")]
    pub default_instances: u32,
}

impl Default for Scale {
    fn default() -> Self {
        Self {
            default_instances: default_instances(),
        }
    }
}

fn default_dockerfile() -> String {
    "./Dockerfile".to_string()
}

fn default_instances() -> u32 {
    1
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("invalid yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("manifest must declare a non-empty `name`")]
    MissingName,
    #[error("manifest must declare a non-empty `domain`")]
    MissingDomain,
    #[error("manifest must declare at least one port")]
    NoPorts,
}

impl Manifest {
    /// Parse and validate a manifest from raw YAML.
    pub fn parse(raw: &str) -> Result<Self, ManifestError> {
        let m: Manifest = serde_yaml::from_str(raw)?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<(), ManifestError> {
        if self.name.trim().is_empty() {
            return Err(ManifestError::MissingName);
        }
        if self.domain.trim().is_empty() {
            return Err(ManifestError::MissingDomain);
        }
        if self.ports.is_empty() {
            return Err(ManifestError::NoPorts);
        }
        Ok(())
    }

    /// The port the app listens on inside the container (first declared port).
    pub fn primary_port(&self) -> u16 {
        self.ports.first().map(|p| p.container).unwrap_or(80)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_manifest() {
        let yaml = r#"
name: my-app
dockerfile: ./Dockerfile
domain: my-app
ports:
  - container: 3000
env:
  NODE_ENV: production
resources:
  cpu: "0.5"
  memory: "256m"
healthcheck:
  path: /health
scale:
  default_instances: 2
"#;
        let m = Manifest::parse(yaml).unwrap();
        assert_eq!(m.name, "my-app");
        assert_eq!(m.primary_port(), 3000);
        assert_eq!(m.scale.default_instances, 2);
        assert_eq!(m.env.get("NODE_ENV").unwrap(), "production");
    }

    #[test]
    fn applies_defaults() {
        let yaml = r#"
name: tiny
domain: tiny
ports:
  - container: 8080
"#;
        let m = Manifest::parse(yaml).unwrap();
        assert_eq!(m.dockerfile, "./Dockerfile");
        assert_eq!(m.scale.default_instances, 1);
    }

    #[test]
    fn rejects_missing_ports() {
        let yaml = "name: x\ndomain: x\n";
        assert!(matches!(
            Manifest::parse(yaml),
            Err(ManifestError::NoPorts)
        ));
    }
}
