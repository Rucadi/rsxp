// SPDX-License-Identifier: GPL-2.0-or-later
//! The stable contract shared by installed FANPLAY extensions and their hosts.
//!
//! This crate intentionally contains no Rhai, egui, emulator-core, or
//! filesystem types. It is suitable for native, headless, and wasm hosts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Major version of the extension ABI. Hosts reject packages with a newer
/// major version instead of attempting a best-effort load.
pub const EXTENSION_API_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ExtensionManifest {
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub entry: EntryPoints,
    #[serde(default)]
    pub compatibility: Compatibility,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub operations: Vec<OperationDeclaration>,
    #[serde(default)]
    pub panels: Vec<PanelDeclaration>,
}

impl ExtensionManifest {
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.api_version == 0 || self.api_version > EXTENSION_API_VERSION {
            return Err(ManifestError::UnsupportedApiVersion(self.api_version));
        }
        if !valid_id(&self.id) {
            return Err(ManifestError::InvalidId(self.id.clone()));
        }
        if self.name.trim().is_empty() || self.version.trim().is_empty() {
            return Err(ManifestError::MissingIdentity);
        }
        for operation in &self.operations {
            if !valid_id(&operation.id) {
                return Err(ManifestError::InvalidId(operation.id.clone()));
            }
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntryPoints {
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub ui: Option<String>,
    /// Optional Ruby source bundled with the package and exposed to Rhai only
    /// as an in-memory string. It is never loaded from the game filesystem.
    #[serde(default)]
    pub ruby: Option<String>,
}

fn default_main() -> String {
    "main.rhai".into()
}

impl Default for EntryPoints {
    fn default() -> Self {
        Self {
            main: default_main(),
            ui: None,
            ruby: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Compatibility {
    #[serde(default)]
    pub cores: Vec<String>,
    #[serde(default)]
    pub games: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OperationDeclaration {
    pub id: String,
    #[serde(default = "default_operation_version")]
    pub version: u32,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub mutates_state: bool,
    #[serde(default)]
    pub route_aliases: Vec<RouteAlias>,
    #[serde(default)]
    pub handler: Option<String>,
}

fn default_operation_version() -> u32 {
    1
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RouteAlias {
    pub method: String,
    pub path: String,
    /// Ordered argument names extracted from path parameters or query values.
    /// The transport remains generic; the extension's Ruby dispatcher owns
    /// validation and defaulting.
    #[serde(default)]
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PanelDeclaration {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A request crossing the extension/core boundary. `generation` makes a
/// reply from a previous game session unrouteable after a reset or reload.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationRequest {
    pub id: u64,
    pub generation: u64,
    pub operation: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OperationResponse {
    pub id: u64,
    pub generation: u64,
    pub result: Result<Value, OperationError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OperationError {
    pub code: String,
    pub message: String,
}

impl OperationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Declarative UI emitted by an extension. Hosts render this without running
/// extension code from inside an egui layout pass.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct UiDescription {
    pub panel_id: String,
    pub title: String,
    #[serde(default)]
    pub nodes: Vec<UiNode>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum UiNode {
    Label {
        id: String,
        text: String,
    },
    Button {
        id: String,
        label: String,
    },
    Checkbox {
        id: String,
        label: String,
        value: bool,
    },
    TextInput {
        id: String,
        label: String,
        value: String,
    },
    Slider {
        id: String,
        label: String,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        suffix: String,
    },
    Select {
        id: String,
        label: String,
        value: String,
        options: Vec<UiOption>,
    },
    Group {
        id: String,
        title: String,
        children: Vec<UiNode>,
    },
    Row {
        id: String,
        children: Vec<UiNode>,
    },
    Scroll {
        id: String,
        children: Vec<UiNode>,
    },
    Loading {
        id: String,
        label: String,
    },
    Table {
        id: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UiOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct UiEvent {
    pub panel_id: String,
    pub widget_id: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExtensionEvent {
    Activated,
    OperationResponse(OperationResponse),
    Subscription { topic: String, payload: Value },
    Ui(UiEvent),
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionLimits {
    pub max_operations: u64,
    pub max_call_levels: usize,
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    pub max_requests: usize,
    pub max_ui_nodes: usize,
}

impl Default for ExtensionLimits {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_call_levels: 32,
            max_string_size: 256 * 1024,
            max_array_size: 4_096,
            max_map_size: 4_096,
            max_requests: 256,
            max_ui_nodes: 4_096,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ManifestError {
    UnsupportedApiVersion(u32),
    InvalidId(String),
    MissingIdentity,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedApiVersion(version) => {
                write!(f, "unsupported extension API version {version}")
            }
            Self::InvalidId(id) => write!(f, "invalid extension identifier {id:?}"),
            Self::MissingIdentity => f.write_str("extension name and version are required"),
        }
    }
}

impl std::error::Error for ManifestError {}
