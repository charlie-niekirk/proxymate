//! Proxymate's colour tokens for the native app. `build.rs` reads
//! `apps/desktop/ui/src/styles/tokens.css` (OKLCH) at build time, so the
//! Tauri UI and the native app share one source until G16 moves the file
//! here. [`DARK`] and [`LIGHT`] hold every colour token as sRGB; the `kit`
//! feature maps them onto gpui-kit's theme ([`kit`]).
//!
//! The contrast rules of `contrast.test.ts` are ported in `contrast.rs`:
//! every text token reaches 4.5:1 on what it is drawn on, in both themes.

pub mod css;
#[cfg(feature = "kit")]
pub mod kit;

#[cfg(test)]
mod contrast;

/// A gamma-encoded sRGB colour with straight alpha, each channel 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// `0xRRGGBBAA`, for debugging and tests.
    pub fn to_u32(self) -> u32 {
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
        byte(self.r) << 24 | byte(self.g) << 16 | byte(self.b) << 8 | byte(self.a)
    }
}

/// Light or dark: which [`Tokens`] apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// The tokens of this mode on this platform (macOS gets a transparent
    /// sidebar, which shows the window's vibrancy).
    pub fn tokens(self) -> Tokens {
        let tokens = match self {
            Mode::Light => LIGHT,
            Mode::Dark => DARK,
        };
        if cfg!(target_os = "macos") {
            tokens.macos()
        } else {
            tokens
        }
    }

    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// `--radius` (0.375rem at 16 px).
pub const RADIUS_PX: f32 = 6.0;

include!(concat!(env!("OUT_DIR"), "/tokens.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_token_is_baked_into_both_themes() {
        assert!(NAMES.len() > 150, "{} tokens", NAMES.len());
        for name in NAMES {
            assert!(DARK.get(name).is_some() && LIGHT.get(name).is_some(), "{name}");
        }
        assert_eq!(DARK.get("--not-a-token"), None);
    }

    #[test]
    fn var_references_resolve_to_their_targets() {
        assert_eq!(DARK.m_options, DARK.m_head);
        assert_eq!(LIGHT.card_foreground, LIGHT.foreground);
        assert_ne!(DARK.background, LIGHT.background);
        // The light block doesn't redeclare --m-other: it inherits the
        // reference, which then resolves against the light --m-connect.
        assert_eq!(LIGHT.m_other, LIGHT.m_connect);
    }

    #[test]
    fn macos_sidebar_is_transparent() {
        let (dark, mac) = (std::hint::black_box(DARK), std::hint::black_box(DARK).macos());
        assert_eq!(mac.sidebar.a, 0.0);
        assert!(dark.sidebar.a > 0.99);
    }
}
