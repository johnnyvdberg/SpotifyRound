//! Album artwork pipeline: download, decode, crop to a circle, derive an accent
//! colour, and cache. Images are built on the UI thread from the decoded pixel
//! buffer because `slint::Image` is not `Send`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use image::imageops::FilterType;
use image::{Rgba, RgbaImage};
use slint::{Rgba8Pixel, SharedPixelBuffer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn color(self) -> slint::Color {
        slint::Color::from_rgb_u8(self.r, self.g, self.b)
    }

    pub fn with_alpha(self, alpha: u8) -> slint::Color {
        slint::Color::from_argb_u8(alpha, self.r, self.g, self.b)
    }

    /// Push a colour towards something that looks good as a UI accent.
    pub fn vivid(self) -> Self {
        let (hue, saturation, lightness) = self.to_hsl();
        Self::from_hsl(
            hue,
            saturation.clamp(0.45, 0.92),
            lightness.clamp(0.5, 0.66),
        )
    }

    pub fn to_hsl(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let lightness = (max + min) / 2.0;
        if (max - min).abs() < f32::EPSILON {
            return (0.0, 0.0, lightness);
        }
        let delta = max - min;
        let saturation = if lightness > 0.5 {
            delta / (2.0 - max - min)
        } else {
            delta / (max + min)
        };
        let hue = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        };
        (hue * 60.0, saturation, lightness)
    }

    pub fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> Self {
        let hue = hue.rem_euclid(360.0) / 60.0;
        let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let x = c * (1.0 - (hue % 2.0 - 1.0).abs());
        let m = lightness - c / 2.0;
        let (r, g, b) = match hue as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        Self::new(
            ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
            ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
            ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        )
    }

    fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Self::new(
            mix(self.r, other.r),
            mix(self.g, other.g),
            mix(self.b, other.b),
        )
    }

    fn lighten(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, s, (l + amount).min(1.0))
    }

    fn darken(self, amount: f32) -> Self {
        let (h, s, l) = self.to_hsl();
        Self::from_hsl(h, s, (l - amount).max(0.0))
    }
}

/// Decoded artwork, ready to be turned into a `slint::Image` on the UI thread.
pub struct Art {
    pixels: SharedPixelBuffer<Rgba8Pixel>,
    accent: Rgb,
}

impl Art {
    pub fn image(&self) -> slint::Image {
        slint::Image::from_rgba8(self.pixels.clone())
    }

    pub fn accent(&self) -> Rgb {
        self.accent
    }
}

/// Downloads and caches artwork.
pub struct Artwork {
    http: reqwest::Client,
    cache: Mutex<HashMap<String, Arc<Art>>>,
}

impl Artwork {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("spotify-round/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .context("building the artwork HTTP client")?;
        Ok(Self {
            http,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Load artwork from an HTTP(S) URL or from a `mock://cover/<hue>/<seed>` URL.
    ///
    /// `corner` is the radius, in decoded pixels, of the softening baked into the
    /// alpha channel while the pixels are prepared off the UI thread.
    pub async fn load(&self, url: &str, size: u32, corner: u32) -> Result<Arc<Art>> {
        if let Some(hit) = self.cached(url, size, corner) {
            return Ok(hit);
        }
        let key = cache_key(url, size, corner);

        let art = if let Some(spec) = url.strip_prefix("mock://cover/") {
            let (hue, seed) = parse_mock_spec(spec);
            tokio::task::spawn_blocking(move || mock_cover(hue, seed, size, corner))
                .await
                .context("cover task")??
        } else {
            let response = self
                .http
                .get(url)
                .send()
                .await
                .with_context(|| format!("downloading artwork from {url}"))?
                .error_for_status()
                .with_context(|| format!("downloading artwork from {url}"))?;
            let bytes = response.bytes().await.context("reading artwork")?;
            tokio::task::spawn_blocking(move || decode(&bytes, size, corner))
                .await
                .context("artwork task")??
        };

        let art = Arc::new(art);
        let mut cache = self.cache.lock().expect("artwork cache");
        // Roughly 10–20 MB of decoded pixels; enough for a session's worth of covers.
        if cache.len() >= 40 {
            cache.clear();
        }
        cache.insert(key, art.clone());
        Ok(art)
    }

    /// Artwork that is already decoded, without waiting. Lists use it to fill
    /// their rows in one go instead of once per cover as the loads complete.
    pub fn cached(&self, url: &str, size: u32, corner: u32) -> Option<Arc<Art>> {
        self.cache
            .lock()
            .expect("artwork cache")
            .get(&cache_key(url, size, corner))
            .cloned()
    }

    /// Render a QR code onto a white canvas.
    pub fn qr(text: &str) -> Result<Art> {
        const SIDE: u32 = 320;
        const QUIET: usize = 4;

        let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
            .context("encoding the authorization URL as a QR code")?;
        let modules = code.width();
        let total = modules + QUIET * 2;
        let step = SIDE as f32 / total as f32;

        let mut canvas = RgbaImage::from_pixel(SIDE, SIDE, Rgba([255, 255, 255, 255]));
        let colors = code.to_colors();
        for y in 0..modules {
            for x in 0..modules {
                if colors[y * modules + x] != qrcode::Color::Dark {
                    continue;
                }
                let x0 = (((x + QUIET) as f32) * step).round() as u32;
                let x1 = (((x + QUIET + 1) as f32) * step).round() as u32;
                let y0 = (((y + QUIET) as f32) * step).round() as u32;
                let y1 = (((y + QUIET + 1) as f32) * step).round() as u32;
                for py in y0..y1 {
                    for px in x0..x1 {
                        canvas.put_pixel(px, py, Rgba([18, 18, 22, 255]));
                    }
                }
            }
        }

        Ok(Art {
            pixels: to_buffer(canvas),
            accent: Rgb::new(30, 215, 96),
        })
    }
}

fn cache_key(url: &str, size: u32, corner: u32) -> String {
    format!("{url}|{size}|{corner}")
}

fn decode(bytes: &[u8], size: u32, corner: u32) -> Result<Art> {
    let image = image::load_from_memory(bytes).context("decoding the artwork")?;
    let resized = image.resize_to_fill(size, size, FilterType::Triangle);
    let mut rgba = resized.to_rgba8();
    let accent = accent_of(&rgba);
    mask_rounded(&mut rgba, corner);
    Ok(Art {
        pixels: to_buffer(rgba),
        accent,
    })
}

fn to_buffer(image: RgbaImage) -> SharedPixelBuffer<Rgba8Pixel> {
    let (width, height) = image.dimensions();
    SharedPixelBuffer::clone_from_slice(image.as_raw(), width, height)
}

fn accent_of(image: &RgbaImage) -> Rgb {
    let total = (image.width() * image.height()) as usize;
    let step = (total / 4096).max(1);
    let (mut r, mut g, mut b, mut count) = (0u64, 0u64, 0u64, 0u64);
    for pixel in image.pixels().step_by(step) {
        if pixel[3] < 200 {
            continue;
        }
        r += pixel[0] as u64;
        g += pixel[1] as u64;
        b += pixel[2] as u64;
        count += 1;
    }
    if count == 0 {
        return Rgb::new(30, 215, 96);
    }
    Rgb::new((r / count) as u8, (g / count) as u8, (b / count) as u8).vivid()
}

/// Fade the pixels outside a rounded rectangle to transparent.
///
/// Uses the signed distance to the shape so the corners are anti-aliased; a
/// radius of half the size produces a circle, zero leaves the image alone.
fn mask_rounded(image: &mut RgbaImage, radius: u32) {
    let (width, height) = image.dimensions();
    let radius = radius.min(width.min(height) / 2);
    if radius == 0 {
        return;
    }
    let r = radius as f32;
    let half_w = width as f32 / 2.0;
    let half_h = height as f32 / 2.0;

    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let px = x as f32 + 0.5 - half_w;
        let py = y as f32 + 0.5 - half_h;
        let qx = px.abs() - (half_w - r);
        let qy = py.abs() - (half_h - r);
        let outside = qx.max(0.0).hypot(qy.max(0.0));
        let distance = outside + qx.max(qy).min(0.0) - r;
        let coverage = (0.5 - distance).clamp(0.0, 1.0);
        pixel[3] = (pixel[3] as f32 * coverage) as u8;
    }
}

fn parse_mock_spec(spec: &str) -> (u16, usize) {
    let mut parts = spec.split('/');
    let hue = parts.next().unwrap_or("0").parse::<u16>().unwrap_or(0) % 360;
    let seed = parts.next().unwrap_or("0").parse::<usize>().unwrap_or(0);
    (hue, seed)
}

/// Deterministic, colourful placeholder artwork for the demo backend.
fn mock_cover(hue: u16, seed: usize, size: u32, corner: u32) -> Result<Art> {
    let start = Rgb::from_hsl(hue as f32, 0.58, 0.36);
    let end = Rgb::from_hsl((hue as f32 + 52.0) % 360.0, 0.72, 0.7);
    let phase = (seed as f32) * 0.8;
    let size = size.max(8);

    let mut image = RgbaImage::new(size, size);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let u = x as f32 / size as f32;
        let v = y as f32 / size as f32;
        let t = (u * 0.62 + v * 0.38).clamp(0.0, 1.0);
        let streak = ((u * 5.5 + v * 2.0 + phase).sin() + 1.0) * 0.04;
        let distance = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * 1.414;
        let colour = start.lerp(end, t).lighten(streak).darken(distance * 0.22);
        *pixel = Rgba([colour.r, colour.g, colour.b, 255]);
    }

    mask_rounded(&mut image, corner);

    Ok(Art {
        pixels: to_buffer(image),
        accent: start.vivid(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_round_trips() {
        for &(r, g, b) in &[(18u8, 52u8, 86u8), (255, 0, 0), (0, 128, 64), (10, 10, 10)] {
            let colour = Rgb::new(r, g, b);
            let (h, s, l) = colour.to_hsl();
            let back = Rgb::from_hsl(h, s, l);
            assert!(
                (back.r as i32 - r as i32).abs() <= 1
                    && (back.g as i32 - g as i32).abs() <= 1
                    && (back.b as i32 - b as i32).abs() <= 1,
                "{colour:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn accents_become_vivid() {
        let grey = Rgb::new(120, 120, 120).vivid();
        let (_, saturation, lightness) = grey.to_hsl();
        assert!(saturation >= 0.44, "{saturation}");
        assert!((0.49..=0.67).contains(&lightness), "{lightness}");
    }

    #[test]
    fn rounded_mask_clears_only_the_corners() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([200, 100, 50, 255]));
        mask_rounded(&mut image, 16);
        assert_eq!(image.get_pixel(0, 0)[3], 0, "corner is cut");
        assert_eq!(image.get_pixel(0, 32)[3], 255, "edges stay filled");
        assert_eq!(image.get_pixel(32, 32)[3], 255);
    }

    #[test]
    fn a_zero_radius_leaves_the_image_alone() {
        let mut image = RgbaImage::from_pixel(16, 16, Rgba([200, 100, 50, 255]));
        mask_rounded(&mut image, 0);
        assert_eq!(image.get_pixel(0, 0)[3], 255);
    }

    #[test]
    fn a_full_radius_rounds_to_a_circle() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([200, 100, 50, 255]));
        mask_rounded(&mut image, 64);
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert_eq!(
            image.get_pixel(0, 16)[3],
            0,
            "a circle has no straight edge to fill"
        );
        assert_eq!(image.get_pixel(32, 32)[3], 255);
    }

    #[test]
    fn mock_covers_are_square_and_opaque_in_the_middle() {
        let art = mock_cover(210, 3, 96, 7).unwrap();
        assert_eq!(art.pixels.width(), 96);
        let buffer = art.pixels.as_slice();
        assert_eq!(buffer[48 * 96 + 48].a, 255);
        assert_eq!(buffer[0].a, 0);
    }

    #[test]
    fn decodes_http_artwork_and_masks_it() {
        let source = RgbaImage::from_pixel(64, 64, Rgba([200, 60, 40, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(source)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();

        let art = decode(&bytes, 32, 4).unwrap();
        let pixels = art.pixels.as_slice();
        assert_eq!(art.pixels.width(), 32);
        assert_eq!(pixels[0].a, 0, "corners are transparent");
        assert_eq!(pixels[16 * 32].a, 255, "the left edge stays opaque");
        assert_eq!(pixels[16 * 32 + 16].a, 255, "centre is opaque");
        assert!(art.accent.r > art.accent.b, "accent keeps the hue");
    }

    #[test]
    fn qr_codes_are_generated() {
        let art = Artwork::qr("https://accounts.spotify.com/authorize?client_id=abc").unwrap();
        assert_eq!(art.pixels.width(), 320);
        assert_eq!(art.pixels.height(), 320);
    }
}
