use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    #[default]
    Normal,
    Left,
    Right,
    Inverted,
}

impl Rotation {
    pub fn as_xrandr(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Left => "left",
            Self::Right => "right",
            Self::Inverted => "inverted",
        }
    }

    pub fn from_xrandr(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "normal" => Some(Self::Normal),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "inverted" => Some(Self::Inverted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalSize {
    pub width_mm: u32,
    pub height_mm: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRate {
    pub value: f64,
    pub current: bool,
    pub preferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rates: Vec<RefreshRate>,
    pub current: bool,
    pub preferred: bool,
}

impl DisplayMode {
    pub fn current_rate(&self) -> Option<f64> {
        self.refresh_rates
            .iter()
            .find(|rate| rate.current)
            .map(|rate| rate.value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentDisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub name: String,
    pub connected: bool,
    pub enabled: bool,
    pub primary: bool,
    pub current_mode: Option<CurrentDisplayMode>,
    pub position: Option<Position>,
    pub rotation: Rotation,
    pub physical_size: Option<PhysicalSize>,
    pub modes: Vec<DisplayMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeSelection {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: Option<f64>,
}

impl ModeSelection {
    pub fn resolution(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayConfiguration {
    pub output: String,
    pub enabled: bool,
    pub mode: Option<ModeSelection>,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub rotation: Rotation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisplayConfigurationRequest {
    pub displays: Vec<DisplayConfiguration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationResponse {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub displays: Vec<Display>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingInfo {
    pub token: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaysResponse {
    pub displays: Vec<Display>,
    pub pending: Option<PendingInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResponse {
    pub success: bool,
    pub confirmation_required: bool,
    pub token: Option<String>,
    pub expires_at: Option<u64>,
    pub message: String,
    pub displays: Vec<Display>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationResponse {
    pub success: bool,
    pub message: String,
    pub displays: Vec<Display>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatus {
    pub application: String,
    pub version: String,
    pub os_name: String,
    pub os_version: Option<String>,
    pub arch_linux: bool,
    pub session_type: String,
    pub display: Option<String>,
    pub xrandr_available: bool,
    pub can_modify: bool,
    pub connected_displays: Option<usize>,
    pub pending_confirmation: bool,
    pub message: String,
}
