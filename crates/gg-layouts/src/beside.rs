//! The lights beside a laptop's keys, from its other controller in GG: the
//! touchpad's rows and the logo, zones over a picture of the lid and the
//! deck side by side, the lid as seen from behind.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use ryukaze_core::{
    layout::{Bounds, Key, Logo, Touchpad},
    lightbar,
};

use crate::{asar::Archive, boxes, css, fit};

/// What a laptop has beside its keys, as far as GG shows it.
#[derive(Debug, Default, PartialEq)]
pub struct Beside {
    pub touchpad: Option<Touchpad>,
    /// The deck, where the touchpad is found on it.
    pub deck: Option<Bounds>,
    pub logo: Option<Logo>,
    /// Why one or the other is left out.
    pub notes: Vec<String>,
}

/// A zone's box in the picture's pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Zone {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Zone {
    fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    fn within(&self, part: &boxes::Patch) -> bool {
        let (x, y) = self.centre();
        part.x0 as f32 <= x && x < part.x1 as f32 && part.y0 as f32 <= y && y < part.y1 as f32
    }
}

fn round(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

/// From `device`'s zones and picture, its style sheet `sheet` in
/// `archive`, the keys laid out as `keys` with gaps `gap` in key widths.
pub fn measure(
    device: &css::Device,
    archive: &Archive,
    sheet: &str,
    keys: &[Key],
    gap: (f32, f32),
) -> Result<Beside> {
    ensure!(!device.zones.is_empty(), "no zones of light");
    let picture = device
        .pictures
        .iter()
        .find(|picture| picture.region.is_none())
        .or(device.pictures.first())
        .context("no picture")?;
    let path = crate::beside(sheet, &picture.url).context("a picture elsewhere")?;
    let image = boxes::decode(&archive.read(&path)?).with_context(|| path.clone())?;
    let (drawn_w, drawn_h) = picture
        .drawn(image.width, image.height)
        .context("no size the picture is drawn at")?;
    let scale = (image.width as f32 / drawn_w, image.height as f32 / drawn_h);
    let zones: BTreeMap<u16, Zone> = device
        .zones
        .iter()
        .map(|(&code, zone)| {
            let zone = Zone {
                x: zone.left * scale.0,
                y: zone.top * scale.1,
                w: zone.width * scale.0,
                h: zone.height * scale.1,
            };
            (code, zone)
        })
        .collect();

    let parts = image.parts();
    let deck = parts
        .iter()
        .max_by_key(|part| image.dark_keys(part).len())
        .context("no deck in the picture")?;
    let lid = parts.iter().find(|part| *part != deck);
    let mut beside = Beside::default();

    let on_deck: Vec<Zone> = zones
        .values()
        .filter(|zone| zone.within(deck))
        .copied()
        .collect();
    match touchpad(&image, deck, &on_deck, keys, gap) {
        Ok((pad, on)) => (beside.touchpad, beside.deck) = (Some(pad), Some(on)),
        Err(err) => beside.notes.push(format!("no touchpad: {err:#}")),
    }
    let on_lid: Vec<Zone> = lid
        .map(|lid| {
            zones
                .values()
                .filter(|zone| zone.within(lid))
                .copied()
                .collect()
        })
        .unwrap_or_default();
    match (lid, on_lid.as_slice()) {
        (Some(lid), [zone]) => beside.logo = Some(logo(lid, zone)),
        (None, _) => beside.notes.push("no logo: no lid in the picture".into()),
        (_, zones) => beside
            .notes
            .push(format!("no logo: {} zones on the lid", zones.len())),
    }
    Ok(beside)
}

/// The logo on the lid as seen from the keys: GG shows the lid from
/// behind, so across is the other way.
fn logo(lid: &boxes::Patch, zone: &Zone) -> Logo {
    let (width, height) = (lid.width() as f32, lid.height() as f32);
    let behind = (zone.x - lid.x0 as f32) / width;
    let w = zone.w / width;
    Logo {
        lid: round(width / height),
        x: round(1.0 - behind - w),
        y: round((zone.y - lid.y0 as f32) / height),
        w: round(w),
        h: round(zone.h / height),
    }
}

/// The touchpad, its zones its rows from the keys down, and the deck, in
/// key widths: the keys drawn on the deck fitted to `keys` for the scale
/// and the shift.
fn touchpad(
    image: &boxes::Picture,
    deck: &boxes::Patch,
    zones: &[Zone],
    keys: &[Key],
    gap: (f32, f32),
) -> Result<(Touchpad, Bounds)> {
    let mut rows = zones.to_vec();
    rows.sort_by(|a, b| a.y.total_cmp(&b.y));
    let Some(first) = rows.first() else {
        bail!("no zones on the deck");
    };
    ensure!(
        rows.len() <= usize::from(lightbar::BAR_ROWS),
        "{} zones on the deck, more than a touchpad's rows",
        rows.len()
    );
    let near = |a: f32, b: f32| (a - b).abs() <= 1.5;
    let stacked = rows
        .iter()
        .all(|row| near(row.x, first.x) && near(row.w, first.w))
        && rows
            .windows(2)
            .all(|two| two[1].y + 1.5 >= two[0].y + two[0].h);
    ensure!(stacked, "zones on the deck that aren't a touchpad's rows");
    let last = rows[rows.len() - 1];

    let caps: BTreeMap<u16, (f32, f32)> = keys
        .iter()
        .map(|key| {
            let centre = (key.x + (key.w - gap.0) / 2.0, key.y + (key.h - gap.1) / 2.0);
            (u16::from(key.hid), centre)
        })
        .collect();
    let drawn = image.dark_keys(deck);
    let fit = fit::fit(&caps, &drawn, image.width, image.height, None)
        .context("no keys drawn on the deck")?;
    ensure!(
        fit.placed.len() * 4 >= caps.len() * 3,
        "only {} of {} keys found on the deck",
        fit.placed.len(),
        caps.len()
    );
    let fit::Transform { at, scale } = fit.transform;
    let pad = Touchpad {
        x: round((first.x - at.0) / scale.0),
        y: round((first.y - at.1) / scale.1),
        w: round(first.w / scale.0),
        h: round((last.y + last.h - first.y) / scale.1),
        rows: rows.len() as u8,
    };
    let on = Bounds {
        x: round((deck.x0 as f32 - at.0) / scale.0),
        y: round((deck.y0 as f32 - at.1) / scale.1),
        w: round(deck.width() as f32 / scale.0),
        h: round(deck.height() as f32 / scale.1),
    };
    Ok((pad, on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_seen_from_the_keys() {
        // As GG's Titan 18 HX Dragon Edition has it.
        let lid = boxes::Patch {
            x0: 0,
            y0: 0,
            x1: 788,
            y1: 525,
        };
        let zone = Zone {
            x: 620.0,
            y: 186.0,
            w: 90.0,
            h: 108.0,
        };
        let logo = logo(&lid, &zone);
        assert_eq!(
            logo,
            Logo {
                lid: 1.501,
                x: 0.099,
                y: 0.354,
                w: 0.114,
                h: 0.206
            }
        );
        assert!((logo.x + logo.w / 2.0 - 0.156).abs() < 0.001);
    }
}
