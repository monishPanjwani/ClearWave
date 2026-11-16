//! Semantic color palette tokens, Omarchy live following, and bundled theme menu.
//!
//! Color values come from data files only. This module never executes theme
//! scripts, sources shell configuration, or modifies the desktop's active theme.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A parsed hex color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse `#RRGGBB` (case-insensitive). Returns `None` for malformed input.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    pub fn to_color32(self) -> eframe::egui::Color32 {
        eframe::egui::Color32::from_rgb(self.r, self.g, self.b)
    }

    /// Relative luminance per WCAG 2.1.
    pub fn luminance(self) -> f64 {
        fn channel(c: u8) -> f64 {
            let v = c as f64 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// WCAG 2.1 contrast ratio between two colors, always >= 1.0.
    pub fn contrast(self, other: Rgb) -> f64 {
        let l1 = self.luminance();
        let l2 = other.luminance();
        if l1 > l2 {
            (l1 + 0.05) / (l2 + 0.05)
        } else {
            (l2 + 0.05) / (l1 + 0.05)
        }
    }

    /// Lighten or darken to achieve at least `min_ratio` contrast against `bg`.
    /// Preserves hue direction; returns self if already passing.
    pub fn ensure_contrast(self, bg: Rgb, min_ratio: f64) -> Rgb {
        if self.contrast(bg) >= min_ratio {
            return self;
        }
        let bg_light = bg.luminance() > 0.179;
        // Blend toward white or black depending on background lightness.
        let target: Rgb = if bg_light {
            Rgb::new(0, 0, 0)
        } else {
            Rgb::new(255, 255, 255)
        };
        // Binary search for the minimum blend factor.
        let mut lo = 0.0_f64;
        let mut hi = 1.0_f64;
        let mut best = self;
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            let blended = Rgb::new(
                ((1.0 - mid) * self.r as f64 + mid * target.r as f64).round() as u8,
                ((1.0 - mid) * self.g as f64 + mid * target.g as f64).round() as u8,
                ((1.0 - mid) * self.b as f64 + mid * target.b as f64).round() as u8,
            );
            if blended.contrast(bg) >= min_ratio {
                best = blended;
                hi = mid;
            } else {
                lo = mid;
            }
        }
        best
    }
}

/// Whether a palette is light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

/// Semantic UI tokens derived from a palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub mode: Mode,
    pub background: Rgb,
    pub surface: Rgb,
    pub text: Rgb,
    pub secondary: Rgb,
    pub accent: Rgb,
    pub selection: Rgb,
    pub border: Rgb,
    pub success: Rgb,
    pub warning: Rgb,
    pub error: Rgb,
    pub meter_normal: Rgb,
    pub meter_clip: Rgb,
}

impl Default for Palette {
    fn default() -> Self {
        Self::classic()
    }
}

impl Palette {
    /// The original ClearWave dark/mint palette.
    pub fn classic() -> Self {
        Self {
            mode: Mode::Dark,
            background: Rgb::new(16, 23, 27),
            surface: Rgb::new(25, 34, 39),
            text: Rgb::new(220, 225, 228),
            secondary: Rgb::new(149, 164, 172),
            accent: Rgb::new(100, 222, 187),
            selection: Rgb::new(36, 83, 70),
            border: Rgb::new(36, 49, 54),
            success: Rgb::new(100, 222, 187),
            warning: Rgb::new(244, 177, 123),
            error: Rgb::new(244, 120, 120),
            meter_normal: Rgb::new(100, 222, 187),
            meter_clip: Rgb::new(244, 177, 123),
        }
    }
}

/// Maximum file size for a colors.toml (safety bound).
const MAX_PALETTE_BYTES: u64 = 16384;

/// Parse an Omarchy-format colors.toml into a Palette.
/// Missing fields are derived from available ones with documented defaults.
pub fn parse_colors_toml(content: &str) -> Option<Palette> {
    // Simple TOML key = "value" parser. We only need flat string values.
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, rest)) = line.split_once('=') {
            let key = key.trim();
            let rest = rest.trim();
            let rest = rest.strip_prefix('"')?;
            let (val, tail) = rest.split_once('"')?;
            if !tail.trim().is_empty() && !tail.trim().starts_with('#') {
                return None;
            }
            fields.insert(key.to_owned(), val.to_owned());
        }
    }

    let get = |key: &str| -> Option<Rgb> { fields.get(key).and_then(|v| Rgb::from_hex(v)) };

    let background = get("background")?;
    let mode = match fields.get("mode").map(|s| s.as_str()) {
        Some("light") => Mode::Light,
        Some("dark") => Mode::Dark,
        _ if background.luminance() > 0.179 => Mode::Light,
        _ => Mode::Dark,
    };
    let surface_channel = |c: u8| {
        if mode == Mode::Light {
            (c as f32 * 0.95) as u8
        } else {
            (c as f32 + (255.0 - c as f32) * 0.04) as u8
        }
    };
    let lighter_bg = get("lighter_background").unwrap_or_else(|| {
        Rgb::new(
            surface_channel(background.r),
            surface_channel(background.g),
            surface_channel(background.b),
        )
    });
    let foreground = get("foreground")?;
    let accent = get("accent").unwrap_or(foreground);
    let selection = get("selection").unwrap_or(lighter_bg);
    let muted = get("muted").unwrap_or(Rgb::new(128, 128, 128));
    let dark_foreground = get("dark_foreground").unwrap_or(muted);
    let red = get("red")
        .or_else(|| get("color1"))
        .unwrap_or(Rgb::new(220, 80, 80));
    let yellow = get("yellow")
        .or_else(|| get("color3"))
        .unwrap_or(Rgb::new(220, 180, 80));
    let green = get("green")
        .or_else(|| get("color2"))
        .unwrap_or(Rgb::new(80, 180, 100));

    // Derive semantic border from lighter_background or selection
    let border = lighter_bg;

    // Ensure text contrast is adequate on the background
    let text = foreground
        .ensure_contrast(lighter_bg, 4.5)
        .ensure_contrast(background, 4.5);
    let secondary = dark_foreground
        .ensure_contrast(lighter_bg, 4.5)
        .ensure_contrast(background, 4.5);
    let accent_ok = accent.ensure_contrast(background, 3.0);
    let warning_ok = yellow.ensure_contrast(background, 3.0);
    let error_ok = red.ensure_contrast(background, 3.0);
    let success_ok = green.ensure_contrast(background, 3.0);

    // Surface (card background): use lighter_background
    let surface = lighter_bg;

    Some(Palette {
        mode,
        background,
        surface,
        text,
        secondary,
        accent: accent_ok,
        selection,
        border,
        success: success_ok,
        warning: warning_ok,
        error: error_ok,
        meter_normal: accent_ok,
        meter_clip: warning_ok,
    })
}

/// A named theme available in the menu.
#[derive(Debug, Clone)]
pub struct ThemeEntry {
    pub id: String,
    pub display_name: String,
    pub palette: Palette,
}

/// Well-known theme identifiers.
pub const FOLLOW_OMARCHY: &str = "follow-omarchy";
pub const CLASSIC: &str = "clearwave-classic";

/// Bundled palette data compiled into the binary.
const BUNDLED: &[(&str, &str, &str)] = &[
    (
        "tokyo-night",
        "Tokyo Night",
        include_str!("../assets/palettes/tokyo-night.toml"),
    ),
    (
        "catppuccin",
        "Catppuccin Mocha",
        include_str!("../assets/palettes/catppuccin.toml"),
    ),
    (
        "catppuccin-latte",
        "Catppuccin Latte",
        include_str!("../assets/palettes/catppuccin-latte.toml"),
    ),
    ("nord", "Nord", include_str!("../assets/palettes/nord.toml")),
    (
        "rose-pine",
        "Rosé Pine Dawn",
        include_str!("../assets/palettes/rose-pine.toml"),
    ),
    (
        "gruvbox",
        "Gruvbox",
        include_str!("../assets/palettes/gruvbox.toml"),
    ),
    (
        "everforest",
        "Everforest",
        include_str!("../assets/palettes/everforest.toml"),
    ),
    (
        "flexoki-light",
        "Flexoki Light",
        include_str!("../assets/palettes/flexoki-light.toml"),
    ),
];

/// Return all bundled themes as parsed entries.
pub fn bundled_themes() -> Vec<ThemeEntry> {
    let mut themes = Vec::with_capacity(BUNDLED.len() + 1);
    for &(id, name, content) in BUNDLED {
        if let Some(palette) = parse_colors_toml(content) {
            themes.push(ThemeEntry {
                id: id.to_owned(),
                display_name: name.to_owned(),
                palette,
            });
        }
    }
    themes.push(ThemeEntry {
        id: CLASSIC.to_owned(),
        display_name: "ClearWave Classic".to_owned(),
        palette: Palette::classic(),
    });
    themes
}

/// Find a bundled theme by its stable identifier.
pub fn bundled_by_id(id: &str) -> Option<Palette> {
    if id == CLASSIC {
        return Some(Palette::classic());
    }
    BUNDLED
        .iter()
        .find(|(bid, _, _)| *bid == id)
        .and_then(|(_, _, content)| parse_colors_toml(content))
}

// ---------- Omarchy live detection ----------

/// Locate the Omarchy active palette file. Checks state path first
/// (the path used by omarchy-theme-set), then the config fallback.
/// Honors absolute XDG_STATE_HOME / XDG_CONFIG_HOME.
pub fn omarchy_palette_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;

    // State path: $XDG_STATE_HOME/omarchy/current/theme/colors.toml
    let state_base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"));
    let state_path = state_base.join("omarchy/current/theme/colors.toml");
    if state_path.is_file() {
        return Some(state_path);
    }

    // Config fallback: $XDG_CONFIG_HOME/omarchy/current/theme/colors.toml
    let config_base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let config_path = config_base.join("omarchy/current/theme/colors.toml");
    if config_path.is_file() {
        return Some(config_path);
    }

    None
}

/// Read and parse the active Omarchy palette, if it exists.
/// Returns None on any error (missing file, parse failure, oversized).
pub fn read_omarchy_palette() -> Option<Palette> {
    let path = omarchy_palette_path()?;
    read_palette_file(&path)
}

/// Read and parse a palette file with safety bounds.
pub fn read_palette_file(path: &Path) -> Option<Palette> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_PALETTE_BYTES {
        return None;
    }
    if !meta.is_file() {
        return None;
    }
    let mut content = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_PALETTE_BYTES + 1)
        .read_to_string(&mut content)
        .ok()?;
    if content.len() as u64 > MAX_PALETTE_BYTES {
        return None;
    }
    parse_colors_toml(&content)
}

/// The directory that Omarchy replaces during theme changes.
/// We watch its parent for rename/delete/create events.
pub fn omarchy_watch_dir() -> Option<PathBuf> {
    omarchy_palette_path().and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// Manages theme state for the UI. Handles live following and debouncing.
pub struct ThemeManager {
    /// Current active palette.
    active: Palette,
    /// Previous palette for transition animation.
    previous: Option<(Palette, Instant)>,
    /// All available themes for the menu.
    entries: Vec<ThemeEntry>,
    /// User's selected theme identifier (persisted).
    selected_id: String,
    /// Cached Omarchy palette (for Follow Omarchy mode).
    omarchy_cache: Option<Palette>,
    /// Path being watched.
    watch_path: Option<PathBuf>,
    /// Last modification time seen on the palette file.
    last_mtime: Option<std::time::SystemTime>,
    /// Last time we checked the file.
    last_check: Instant,
    /// Whether Omarchy directories exist at all.
    omarchy_available: bool,
    /// Transition animation progress (0.0 to 1.0).
    transition: f32,
}

const CHECK_INTERVAL: Duration = Duration::from_millis(2000);
const TRANSITION_DURATION: Duration = Duration::from_millis(250);

impl ThemeManager {
    pub fn new(saved_theme_id: Option<&str>) -> Self {
        let entries = bundled_themes();
        let omarchy_palette = read_omarchy_palette();
        let omarchy_available = omarchy_palette.is_some() || omarchy_palette_path().is_some();
        let watch_path = omarchy_palette_path();

        let last_mtime = watch_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()));

        // Determine initial theme
        let (selected_id, active) = match saved_theme_id {
            Some(id) if id == FOLLOW_OMARCHY => {
                let palette = omarchy_palette.clone().unwrap_or_else(Palette::classic);
                (FOLLOW_OMARCHY.to_owned(), palette)
            }
            Some(id) => {
                let palette = bundled_by_id(id).unwrap_or_else(Palette::classic);
                (
                    if bundled_by_id(id).is_some() {
                        id
                    } else {
                        CLASSIC
                    }
                    .to_owned(),
                    palette,
                )
            }
            None => {
                // First launch: follow Omarchy if available, otherwise Classic.
                if let Some(palette) = omarchy_palette.clone() {
                    (FOLLOW_OMARCHY.to_owned(), palette)
                } else {
                    (CLASSIC.to_owned(), Palette::classic())
                }
            }
        };

        Self {
            active,
            previous: None,
            entries,
            selected_id,
            omarchy_cache: omarchy_palette,
            watch_path,
            last_mtime,
            last_check: Instant::now(),
            omarchy_available,
            transition: 1.0,
        }
    }

    /// The current active palette.
    pub fn palette(&self) -> &Palette {
        &self.active
    }

    /// Get a blended color for transition animation.
    /// Returns the target color if no transition is active.
    pub fn transition_progress(&self) -> f32 {
        self.transition
    }

    /// All theme entries for the menu.
    pub fn entries(&self) -> &[ThemeEntry] {
        &self.entries
    }

    /// Current selected theme ID (for persistence).
    pub fn selected_id(&self) -> &str {
        &self.selected_id
    }

    /// Whether Omarchy was detected on this system.
    pub fn omarchy_available(&self) -> bool {
        self.omarchy_available
    }

    /// Whether currently following Omarchy.
    pub fn is_following(&self) -> bool {
        self.selected_id == FOLLOW_OMARCHY
    }

    /// Select a theme by ID. Returns true if the palette changed.
    pub fn select(&mut self, id: &str) -> bool {
        self.selected_id = if id == FOLLOW_OMARCHY || bundled_by_id(id).is_some() {
            id
        } else {
            CLASSIC
        }
        .to_owned();
        let new_palette = if id == FOLLOW_OMARCHY {
            read_omarchy_palette()
                .or_else(|| self.omarchy_cache.clone())
                .unwrap_or_else(Palette::classic)
        } else {
            bundled_by_id(id).unwrap_or_else(Palette::classic)
        };
        self.apply_palette(new_palette)
    }

    /// Poll for Omarchy theme changes. Call from the UI update loop.
    /// Returns true if the palette changed. Debounces reads.
    pub fn poll(&mut self) -> bool {
        // Advance transition
        if self.transition < 1.0 {
            if let Some((_, started)) = &self.previous {
                let elapsed = started.elapsed().as_secs_f32();
                self.transition = (elapsed / TRANSITION_DURATION.as_secs_f32()).min(1.0);
                if self.transition >= 1.0 {
                    self.previous = None;
                }
            }
        }

        if !self.is_following() {
            return false;
        }

        if self.last_check.elapsed() < CHECK_INTERVAL {
            return false;
        }
        self.last_check = Instant::now();

        // Re-check: the palette path may have appeared or moved.
        let current_path = omarchy_palette_path();
        if current_path != self.watch_path {
            self.watch_path = current_path.clone();
            self.omarchy_available = current_path.is_some();
        }

        let Some(path) = &self.watch_path else {
            self.last_mtime = None;
            self.omarchy_available = false;
            return false;
        };

        // Check modification time to avoid unnecessary reads.
        let mtime = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        if mtime == self.last_mtime && self.omarchy_cache.is_some() {
            return false;
        }

        // The file changed (or was replaced). Read it.
        if let Some(palette) = read_palette_file(path) {
            self.omarchy_available = true;
            self.last_mtime = mtime;
            self.omarchy_cache = Some(palette.clone());
            self.apply_palette(palette)
        } else {
            self.omarchy_available = false;
            // Transient missing file during directory replacement.
            // Keep the last valid palette.
            false
        }
    }

    fn apply_palette(&mut self, new: Palette) -> bool {
        if new == self.active {
            return false;
        }
        self.previous = Some((self.active.clone(), Instant::now()));
        self.transition = 0.0;
        self.active = new;
        true
    }

    /// Blend between old and new palette color during transition.
    pub fn blend(&self, get: impl Fn(&Palette) -> Rgb) -> Rgb {
        if self.transition >= 1.0 {
            return get(&self.active);
        }
        if let Some((prev, _)) = &self.previous {
            let old = get(prev);
            let new = get(&self.active);
            let t = self.transition;
            Rgb::new(
                ((1.0 - t) * old.r as f32 + t * new.r as f32).round() as u8,
                ((1.0 - t) * old.g as f32 + t * new.g as f32).round() as u8,
                ((1.0 - t) * old.b as f32 + t * new.b as f32).round() as u8,
            )
        } else {
            get(&self.active)
        }
    }
}

// ---------- Tests ----------
#[cfg(test)]
mod tests {
    use super::*;
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn parse_tokyo_night() {
        let content = include_str!("../assets/palettes/tokyo-night.toml");
        let palette = parse_colors_toml(content).expect("should parse tokyo-night");
        assert_eq!(palette.mode, Mode::Dark);
        assert_eq!(palette.background, Rgb::from_hex("#1a1b26").unwrap());
        assert_eq!(
            palette.accent,
            Rgb::from_hex("#7aa2f7")
                .unwrap()
                .ensure_contrast(palette.background, 3.0)
        );
    }

    #[test]
    fn parse_catppuccin_latte_light() {
        let content = include_str!("../assets/palettes/catppuccin-latte.toml");
        let palette = parse_colors_toml(content).expect("should parse catppuccin-latte");
        assert_eq!(palette.mode, Mode::Light);
    }

    #[test]
    fn parse_all_bundled_palettes() {
        let themes = bundled_themes();
        // 8 Omarchy + 1 Classic
        assert_eq!(themes.len(), 9);
        for theme in &themes {
            let p = &theme.palette;
            // Text must have adequate contrast on background
            assert!(
                p.text.contrast(p.background) >= 4.0,
                "{}: text contrast {:.2} < 4.0",
                theme.id,
                p.text.contrast(p.background)
            );
            // Accent must have some contrast
            assert!(
                p.accent.contrast(p.background) >= 2.5,
                "{}: accent contrast {:.2} < 2.5",
                theme.id,
                p.accent.contrast(p.background)
            );
        }
    }

    #[test]
    fn classic_palette_matches_original_colors() {
        let p = Palette::classic();
        assert_eq!(p.background, Rgb::new(16, 23, 27));
        assert_eq!(p.accent, Rgb::new(100, 222, 187));
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(Rgb::from_hex("#FF00FF"), Some(Rgb::new(255, 0, 255)));
        assert_eq!(Rgb::from_hex("#ff00ff"), Some(Rgb::new(255, 0, 255)));
        assert_eq!(Rgb::from_hex("FF00FF"), None); // missing #
        assert_eq!(Rgb::from_hex("#FFF"), None); // too short
        assert_eq!(Rgb::from_hex("#ééé"), None);
        assert_eq!(Rgb::from_hex("#GGGGGG"), None); // invalid hex
    }

    #[test]
    fn contrast_ratio() {
        let black = Rgb::new(0, 0, 0);
        let white = Rgb::new(255, 255, 255);
        let ratio = black.contrast(white);
        assert!((ratio - 21.0).abs() < 0.1);
        assert!((black.contrast(black) - 1.0).abs() < 0.001);
    }

    #[test]
    fn ensure_contrast_adjusts_faint_color() {
        let dark_bg = Rgb::new(20, 20, 20);
        let faint = Rgb::new(40, 40, 40);
        assert!(faint.contrast(dark_bg) < 2.0);
        let adjusted = faint.ensure_contrast(dark_bg, 4.5);
        assert!(adjusted.contrast(dark_bg) >= 4.5);
    }

    #[test]
    fn ensure_contrast_preserves_passing_color() {
        let bg = Rgb::new(20, 20, 20);
        let good = Rgb::new(200, 200, 200);
        assert!(good.contrast(bg) > 4.5);
        assert_eq!(good.ensure_contrast(bg, 4.5), good);
    }

    #[test]
    fn missing_toml_fields_use_defaults() {
        let minimal = r##"
mode = "dark"
background = "#1a1b26"
foreground = "#a9b1d6"
"##;
        let palette = parse_colors_toml(minimal).expect("should parse minimal");
        assert_eq!(palette.mode, Mode::Dark);
        assert_eq!(palette.background, Rgb::from_hex("#1a1b26").unwrap());
    }

    #[test]
    fn invalid_toml_returns_none() {
        assert!(parse_colors_toml("").is_none());
        assert!(parse_colors_toml("no_background = \"x\"").is_none());
        assert!(
            parse_colors_toml("background = \"not-a-color\"\nforeground = \"#aabbcc\"").is_none()
        );
    }

    #[test]
    fn bundled_by_id_finds_known_themes() {
        assert!(bundled_by_id("tokyo-night").is_some());
        assert!(bundled_by_id("clearwave-classic").is_some());
        assert!(bundled_by_id("nonexistent").is_none());
    }

    #[test]
    fn theme_manager_initial_without_omarchy() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Without Omarchy dirs on the system, should fall back to Classic
        let mut mgr = ThemeManager::new(None);
        // The manager may or may not find Omarchy depending on the test machine.
        // But with an explicit Classic selection it should work:
        mgr.select(CLASSIC);
        assert_eq!(*mgr.palette(), Palette::classic());
        assert_eq!(mgr.selected_id(), CLASSIC);
    }

    #[test]
    fn theme_manager_select_bundled() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut mgr = ThemeManager::new(Some(CLASSIC));
        assert!(mgr.select("nord"));
        let nord = bundled_by_id("nord").unwrap();
        assert_eq!(*mgr.palette(), nord);
    }

    #[test]
    fn theme_manager_select_unknown_falls_back() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mgr = ThemeManager::new(Some("nonexistent-theme-xyz"));
        assert_eq!(*mgr.palette(), Palette::classic());
    }

    #[test]
    fn omarchy_palette_path_respects_xdg_state() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Test with an overridden XDG_STATE_HOME that doesn't exist
        let dir = std::env::temp_dir().join(format!("clearwave-theme-test-{}", std::process::id()));
        let theme_dir = dir.join("omarchy/current/theme");
        std::fs::create_dir_all(&theme_dir).unwrap();
        let path = theme_dir.join("colors.toml");
        std::fs::write(&path, include_str!("../assets/palettes/tokyo-night.toml")).unwrap();

        // Temporarily set XDG_STATE_HOME
        let old = std::env::var_os("XDG_STATE_HOME");
        std::env::set_var("XDG_STATE_HOME", &dir);
        let result = omarchy_palette_path();
        assert_eq!(result.as_deref(), Some(path.as_path()));
        let palette = read_omarchy_palette();
        assert!(palette.is_some());

        // Restore
        match old {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn directory_replacement_debounce() {
        let _guard = ENV_LOCK.lock().unwrap();
        // Simulate theme directory replacement
        let dir =
            std::env::temp_dir().join(format!("clearwave-dir-replace-{}", std::process::id()));
        let theme_dir = dir.join("omarchy/current/theme");
        std::fs::create_dir_all(&theme_dir).unwrap();
        let path = theme_dir.join("colors.toml");
        std::fs::write(&path, include_str!("../assets/palettes/tokyo-night.toml")).unwrap();

        let old = std::env::var_os("XDG_STATE_HOME");
        std::env::set_var("XDG_STATE_HOME", &dir);

        let mut mgr = ThemeManager::new(Some(FOLLOW_OMARCHY));
        assert!(mgr.palette().mode == Mode::Dark);

        // Simulate directory replacement: remove, pause, recreate with light theme
        std::fs::remove_dir_all(&theme_dir).unwrap();
        // Polling during removal should not crash and should keep last valid palette
        mgr.last_check = Instant::now() - Duration::from_secs(10); // force check
        let _changed = mgr.poll();
        // Should keep last valid palette, not crash
        assert!(mgr.palette().mode == Mode::Dark);

        // Recreate with a light theme
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(
            &path,
            include_str!("../assets/palettes/catppuccin-latte.toml"),
        )
        .unwrap();
        mgr.last_check = Instant::now() - Duration::from_secs(10); // force check
        let _changed = mgr.poll();
        assert_eq!(mgr.palette().mode, Mode::Light);
        mgr.select("nord");
        std::fs::write(&path, include_str!("../assets/palettes/gruvbox.toml")).unwrap();
        mgr.last_check = Instant::now() - Duration::from_secs(10);
        mgr.poll();
        assert_eq!(mgr.palette(), &bundled_by_id("nord").unwrap());

        // Restore
        match old {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_palette_rejected() {
        let dir = std::env::temp_dir().join(format!("clearwave-oversize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("colors.toml");
        // Write a file larger than MAX_PALETTE_BYTES
        let content = "x".repeat(MAX_PALETTE_BYTES as usize + 1);
        std::fs::write(&path, content).unwrap();
        assert!(read_palette_file(&path).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
