use std::{env, io, process::Stdio};

use serde::Serialize;
use thiserror::Error;
use tokio::process::Command;

use crate::model::{
    CurrentDisplayMode, Display, DisplayConfiguration, DisplayMode, PhysicalSize, Position,
    RefreshRate, Rotation,
};

#[derive(Debug, Error)]
pub enum XrandrError {
    #[error("未找到 xrandr，请先安装 xorg-xrandr")]
    NotInstalled,
    #[error("当前是 Wayland 会话，第一版仅支持 X11")]
    UnsupportedSession,
    #[error("无法连接 X11 显示服务器：{0}")]
    DisplayUnavailable(String),
    #[error("xrandr 查询失败：{0}")]
    QueryFailed(String),
    #[error("xrandr 应用配置失败：{0}")]
    ApplyFailed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct OsRelease {
    pub name: String,
    pub version: Option<String>,
    pub arch_linux: bool,
}

#[derive(Debug, Clone)]
pub struct XrandrManager {
    binary: String,
}

impl Default for XrandrManager {
    fn default() -> Self {
        Self::new()
    }
}

impl XrandrManager {
    pub fn new() -> Self {
        Self {
            binary: env::var("ARCH_TOOLS_XRANDR")
                .or_else(|_| env::var("ADM_XRANDR"))
                .unwrap_or_else(|_| "xrandr".to_string()),
        }
    }

    pub fn session_type() -> String {
        env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| {
            if env::var_os("DISPLAY").is_some() {
                "x11".to_string()
            } else {
                "unknown".to_string()
            }
        })
    }

    pub fn display_environment() -> Option<String> {
        env::var("DISPLAY").ok()
    }

    pub fn os_release() -> OsRelease {
        let contents = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let mut values = std::collections::HashMap::new();
        for line in contents.lines() {
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.trim(), value.trim().trim_matches('"').to_string());
            }
        }
        let id = values.get("ID").map(String::as_str).unwrap_or_default();
        let id_like = values
            .get("ID_LIKE")
            .map(String::as_str)
            .unwrap_or_default();
        let arch_linux = id.eq_ignore_ascii_case("arch")
            || id_like
                .split_whitespace()
                .any(|value| value.eq_ignore_ascii_case("arch"));
        OsRelease {
            name: values
                .get("PRETTY_NAME")
                .cloned()
                .or_else(|| values.get("NAME").cloned())
                .unwrap_or_else(|| "Linux".to_string()),
            version: values.get("VERSION_ID").cloned(),
            arch_linux,
        }
    }

    pub async fn is_available(&self) -> bool {
        Command::new(&self.binary)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
    }

    pub async fn ensure_usable(&self) -> Result<(), XrandrError> {
        if Self::session_type().eq_ignore_ascii_case("wayland") {
            return Err(XrandrError::UnsupportedSession);
        }
        if !self.is_available().await {
            return Err(XrandrError::NotInstalled);
        }
        Ok(())
    }

    pub async fn query(&self) -> Result<Vec<Display>, XrandrError> {
        let output = Command::new(&self.binary)
            .arg("--query")
            .output()
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    XrandrError::NotInstalled
                } else {
                    XrandrError::DisplayUnavailable(error.to_string())
                }
            })?;

        if !output.status.success() {
            let message = command_error(&output.stderr, output.status.code());
            if message.to_ascii_lowercase().contains("cannot open display")
                || message
                    .to_ascii_lowercase()
                    .contains("unable to open display")
            {
                return Err(XrandrError::DisplayUnavailable(message));
            }
            return Err(XrandrError::QueryFailed(message));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_query_output(&stdout)
    }

    pub async fn apply(&self, configurations: &[DisplayConfiguration]) -> Result<(), XrandrError> {
        let args = build_xrandr_args(configurations);
        let output = Command::new(&self.binary)
            .args(&args)
            .output()
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    XrandrError::NotInstalled
                } else {
                    XrandrError::ApplyFailed(error.to_string())
                }
            })?;

        if !output.status.success() {
            return Err(XrandrError::ApplyFailed(command_error(
                &output.stderr,
                output.status.code(),
            )));
        }
        Ok(())
    }
}

fn command_error(stderr: &[u8], code: Option<i32>) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.is_empty() {
        match code {
            Some(code) => format!("进程退出码 {code}"),
            None => "进程被信号中断".to_string(),
        }
    } else {
        message
    }
}

pub fn build_xrandr_args(configurations: &[DisplayConfiguration]) -> Vec<String> {
    let mut args = Vec::new();
    // Enable/configure active outputs before turning others off to reduce black-screen risk.
    for configuration in configurations
        .iter()
        .filter(|configuration| configuration.enabled)
        .chain(
            configurations
                .iter()
                .filter(|configuration| !configuration.enabled),
        )
    {
        args.push("--output".to_string());
        args.push(configuration.output.clone());
        if !configuration.enabled {
            args.push("--off".to_string());
            continue;
        }

        if let Some(mode) = &configuration.mode {
            args.push("--mode".to_string());
            args.push(mode.resolution());
            if let Some(refresh_rate) = mode.refresh_rate {
                args.push("--rate".to_string());
                args.push(format_rate(refresh_rate));
            }
        } else {
            args.push("--auto".to_string());
        }

        args.push("--pos".to_string());
        args.push(format!("{}x{}", configuration.x, configuration.y));
        args.push("--rotate".to_string());
        args.push(configuration.rotation.as_xrandr().to_string());
        if configuration.primary {
            args.push("--primary".to_string());
        }
    }
    args
}

pub fn format_rate(value: f64) -> String {
    let mut formatted = format!("{value:.3}");
    while formatted.contains('.') && formatted.ends_with('0') {
        formatted.pop();
    }
    if formatted.ends_with('.') {
        formatted.pop();
    }
    formatted
}

pub fn parse_query_output(input: &str) -> Result<Vec<Display>, XrandrError> {
    let mut displays = Vec::new();

    for line in input.lines() {
        if let Some((name, connected, primary)) = parse_output_header(line) {
            let (current_mode, position) = find_geometry(line).map_or((None, None), |geometry| {
                (
                    Some(CurrentDisplayMode {
                        width: geometry.0,
                        height: geometry.1,
                        refresh_rate: None,
                    }),
                    Some(Position {
                        x: geometry.2,
                        y: geometry.3,
                    }),
                )
            });
            let rotation = parse_rotation(line).unwrap_or_default();
            let physical_size = parse_physical_size(line);
            displays.push(Display {
                name,
                connected,
                enabled: current_mode.is_some(),
                primary,
                current_mode,
                position,
                rotation,
                physical_size,
                modes: Vec::new(),
            });
            continue;
        }

        let Some(mode) = parse_mode_line(line) else {
            continue;
        };
        let Some(display) = displays.last_mut() else {
            continue;
        };
        merge_mode(display, mode);
    }

    for display in &mut displays {
        // xrandr reports the rotated geometry for left/right outputs, but mode
        // lines always use the panel's native orientation.
        let rotated = matches!(display.rotation, Rotation::Left | Rotation::Right);
        let current_dimensions = display.current_mode.as_ref().map(|mode| {
            if rotated {
                (mode.height, mode.width)
            } else {
                (mode.width, mode.height)
            }
        });
        for mode in &mut display.modes {
            if Some((mode.width, mode.height)) == current_dimensions {
                mode.current = true;
                if let Some(current_mode) = display.current_mode.as_mut() {
                    current_mode.refresh_rate = mode.current_rate();
                }
            }
            mode.preferred = mode.refresh_rates.iter().any(|rate| rate.preferred);
        }
        if rotated && let Some(current_mode) = display.current_mode.as_mut() {
            std::mem::swap(&mut current_mode.width, &mut current_mode.height);
        }
    }

    Ok(displays)
}

fn parse_output_header(line: &str) -> Option<(String, bool, bool)> {
    if line.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let mut tokens = line.split_whitespace();
    let name = tokens.next()?;
    let state = tokens.next()?;
    if state != "connected" && state != "disconnected" {
        return None;
    }
    let primary = tokens.any(|token| token == "primary");
    Some((name.to_string(), state == "connected", primary))
}

fn parse_resolution_prefix(token: &str) -> Option<(u32, u32, usize)> {
    let bytes = token.as_bytes();
    let x_index = bytes.iter().position(|byte| *byte == b'x')?;
    if x_index == 0 {
        return None;
    }
    let mut end = x_index + 1;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == x_index + 1 {
        return None;
    }
    let width = token[..x_index].parse().ok()?;
    let height = token[x_index + 1..end].parse().ok()?;
    if end < bytes.len() && !matches!(bytes[end], b'i' | b'p' | b'+' | b'-') {
        return None;
    }
    Some((width, height, end))
}

fn parse_geometry(token: &str) -> Option<(u32, u32, i32, i32)> {
    let (width, height, mut index) = parse_resolution_prefix(token)?;
    let (x, next) = parse_signed_number(token, index)?;
    index = next;
    let (y, _) = parse_signed_number(token, index)?;
    Some((width, height, x, y))
}

fn parse_signed_number(value: &str, mut index: usize) -> Option<(i32, usize)> {
    let bytes = value.as_bytes();
    if index >= bytes.len() || !matches!(bytes[index], b'+' | b'-') {
        return None;
    }
    let sign = if bytes[index] == b'-' { -1 } else { 1 };
    index += 1;
    let start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if start == index {
        return None;
    }
    let number: i32 = value[start..index].parse().ok()?;
    Some((number.saturating_mul(sign), index))
}

fn find_geometry(line: &str) -> Option<(u32, u32, i32, i32)> {
    line.split_whitespace().find_map(parse_geometry)
}

/// The active rotation is a standalone token between the geometry and the
/// parenthesised list of supported rotations, e.g.
/// `HDMI-1 connected 1080x1920+0+0 left (normal left inverted right x axis y axis)`.
/// When the output is not rotated xrandr omits the token entirely.
fn parse_rotation(line: &str) -> Option<Rotation> {
    let end = line.find('(').unwrap_or(line.len());
    line[..end]
        .split_whitespace()
        .skip_while(|token| parse_geometry(token).is_none())
        .skip(1)
        .find_map(Rotation::from_xrandr)
}

fn parse_physical_size(line: &str) -> Option<PhysicalSize> {
    let close = line.find(')')?;
    let tokens: Vec<&str> = line[close + 1..].split_whitespace().collect();
    for window in tokens.windows(3) {
        if window[1] != "x" {
            continue;
        }
        let Some(width) = window[0]
            .strip_suffix("mm")
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let Some(height) = window[2]
            .strip_suffix("mm")
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        return Some(PhysicalSize {
            width_mm: width,
            height_mm: height,
        });
    }
    None
}

fn parse_mode_line(line: &str) -> Option<DisplayMode> {
    if !line.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let mut tokens = line.split_whitespace();
    let resolution = tokens.next()?;
    let (width, height, _) = parse_resolution_prefix(resolution)?;
    let mut refresh_rates = Vec::new();
    for token in tokens {
        let numeric_end = token
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit() || *character == '.')
            .last()
            .map_or(0, |(index, character)| index + character.len_utf8());
        if numeric_end == 0 {
            continue;
        }
        let Ok(value) = token[..numeric_end].parse::<f64>() else {
            continue;
        };
        if !value.is_finite() || value <= 0.0 {
            continue;
        }
        let flags = &token[numeric_end..];
        refresh_rates.push(RefreshRate {
            value,
            current: flags.contains('*'),
            preferred: flags.contains('+'),
        });
    }
    Some(DisplayMode {
        width,
        height,
        current: false,
        preferred: false,
        refresh_rates,
    })
}

fn merge_mode(display: &mut Display, incoming: DisplayMode) {
    if let Some(existing) = display
        .modes
        .iter_mut()
        .find(|mode| mode.width == incoming.width && mode.height == incoming.height)
    {
        for incoming_rate in incoming.refresh_rates {
            if let Some(existing_rate) = existing
                .refresh_rates
                .iter_mut()
                .find(|rate| (rate.value - incoming_rate.value).abs() < 0.01)
            {
                existing_rate.current |= incoming_rate.current;
                existing_rate.preferred |= incoming_rate.preferred;
            } else {
                existing.refresh_rates.push(incoming_rate);
            }
        }
    } else {
        display.modes.push(incoming);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DisplayConfiguration, ModeSelection};

    const SAMPLE: &str = r#"Screen 0: minimum 320 x 200, current 2944 x 1080, maximum 8192 x 8192
eDP-1 connected primary 1920x1080+0+0 (normal left inverted right x axis y axis) 309mm x 174mm
   1920x1080     60.01*+  59.93
   1680x1050     59.95
HDMI-1 connected 1024x768+1920+0 (normal left inverted right x axis y axis) 340mm x 190mm
   1920x1080     60.00 +
   1024x768      75.03*  70.07  60.00
DP-1 disconnected (normal left inverted right x axis y axis)
"#;

    #[test]
    fn parses_outputs_modes_and_geometry() {
        let displays = parse_query_output(SAMPLE).expect("sample should parse");
        assert_eq!(displays.len(), 3);
        assert_eq!(displays[0].name, "eDP-1");
        assert!(displays[0].connected);
        assert!(displays[0].enabled);
        assert!(displays[0].primary);
        assert_eq!(displays[0].position.as_ref().unwrap().x, 0);
        assert_eq!(displays[0].modes.len(), 2);
        assert!(displays[0].modes[0].current);
        assert!(displays[0].modes[0].preferred);
        assert_eq!(
            displays[0].current_mode.as_ref().unwrap().refresh_rate,
            Some(60.01)
        );
        assert_eq!(displays[1].position.as_ref().unwrap().x, 1920);
        assert!(!displays[2].connected);
        assert!(!displays[2].enabled);
    }

    #[test]
    fn builds_argument_vector_without_shell_interpolation() {
        let configs = vec![
            DisplayConfiguration {
                output: "HDMI-1".to_string(),
                enabled: true,
                mode: Some(ModeSelection {
                    width: 1920,
                    height: 1080,
                    refresh_rate: Some(60.0),
                }),
                x: 0,
                y: 0,
                primary: true,
                rotation: Rotation::Normal,
            },
            DisplayConfiguration {
                output: "VGA1".to_string(),
                enabled: false,
                mode: None,
                x: 1920,
                y: 0,
                primary: false,
                rotation: Rotation::Normal,
            },
        ];
        assert_eq!(
            build_xrandr_args(&configs),
            vec![
                "--output",
                "HDMI-1",
                "--mode",
                "1920x1080",
                "--rate",
                "60",
                "--pos",
                "0x0",
                "--rotate",
                "normal",
                "--primary",
                "--output",
                "VGA1",
                "--off",
            ]
        );
    }

    #[test]
    fn parses_negative_geometry_for_read_only_state() {
        let displays = parse_query_output(
            "Screen 0: current 2000 x 1000\nHDMI-1 connected 1920x1080+0+0 (normal left inverted right x axis y axis) 300mm x 200mm\nDP-1 connected 1080x1920+1920-10 left (normal left inverted right x axis y axis) 200mm x 100mm\n   1920x1080     60.00*+\n",
        )
        .unwrap();
        assert_eq!(displays[1].position.as_ref().unwrap().y, -10);
        assert_eq!(displays[1].rotation, Rotation::Left);
        assert_eq!(displays[0].rotation, Rotation::Normal);
    }

    #[test]
    fn rotated_output_matches_native_mode_and_keeps_native_dimensions() {
        let displays = parse_query_output(
            "Screen 0: current 3000 x 1920\nHDMI-1 connected 1080x1920+0+0 right (normal left inverted right x axis y axis) 300mm x 200mm\n   1920x1080     60.00*+  50.00\n   1280x720      60.00\n",
        )
        .unwrap();
        let display = &displays[0];
        assert_eq!(display.rotation, Rotation::Right);
        let current = display.current_mode.as_ref().unwrap();
        assert_eq!((current.width, current.height), (1920, 1080));
        assert_eq!(current.refresh_rate, Some(60.0));
        assert!(display.modes[0].current);
        assert!(!display.modes[1].current);
    }

    #[test]
    fn malformed_refresh_token_does_not_drop_mode_line() {
        let displays = parse_query_output(
            "HDMI-1 connected 1920x1080+0+0 (normal)\n   1920x1080     1.2.3  60.00*+\n",
        )
        .unwrap();
        assert_eq!(displays[0].modes.len(), 1);
        assert_eq!(displays[0].modes[0].refresh_rates.len(), 1);
    }

    #[test]
    fn os_release_parser_has_a_safe_fallback() {
        assert!(!XrandrManager::os_release().name.is_empty());
    }
}
