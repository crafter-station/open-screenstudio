use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseMove {
    pub x: f64,
    pub y: f64,
    pub cursor_id: String,
    pub active_modifiers: Vec<String>,
    pub process_time_ms: f64,
    pub unix_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseClick {
    pub x: f64,
    pub y: f64,
    pub button: String,
    pub event_type: String,
    pub click_count: u32,
    pub active_modifiers: Vec<String>,
    pub process_time_ms: f64,
    pub unix_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorInfo {
    pub id: String,
    pub image_path: String,
    pub hotspot_x: f64,
    pub hotspot_y: f64,
    pub width: u32,
    pub height: u32,
}
