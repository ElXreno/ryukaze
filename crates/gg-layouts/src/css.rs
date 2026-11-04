//! What GG's style sheets say of a device: where its key markers sit over
//! its picture, in CSS pixels, and the pictures, one a keyboard region.

use std::collections::BTreeMap;

/// A rule: its selectors, each with its whitespace collapsed, and its
/// declarations in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub selectors: Vec<String>,
    pub declarations: Vec<(String, String)>,
}

/// The size GG's markers are, where a device's rules don't say.
const MARKER_SIZE: f32 = 34.0;

/// A key's marker: its HID usage's box over the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

impl Marker {
    pub fn centre(&self) -> (f32, f32) {
        (self.left + self.width / 2.0, self.top + self.height / 2.0)
    }
}

/// A picture of the device, for a keyboard region or for all.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub region: Option<u32>,
    /// Where it is, beside the style sheet.
    pub url: String,
    /// The CSS pixels it's drawn over, if the rule says.
    pub size: Option<(f32, f32)>,
    /// Or the box it's fitted into whole (`background-size: contain`).
    pub contained: Option<(f32, f32)>,
}

impl Picture {
    /// The CSS pixels a picture `width` by `height` is drawn over.
    pub fn drawn(&self, width: usize, height: usize) -> Option<(f32, f32)> {
        let (width, height) = (width as f32, height as f32);
        self.size.or_else(|| {
            let (w, h) = self.contained?;
            let scale = (w / width).min(h / height);
            Some((width * scale, height * scale))
        })
    }
}

/// A device's markers, its light zones and its pictures.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Device {
    pub markers: BTreeMap<u16, Marker>,
    /// The zones of a controller beside the keys, such as the light bar's
    /// rows and the logo, by number.
    pub zones: BTreeMap<u16, Marker>,
    pub pictures: Vec<Picture>,
}

/// The rules at the top level of a style sheet; those in at-rules, for
/// other screens or fonts, left out.
pub fn rules(text: &str) -> Vec<Rule> {
    let text = without_comments(text);
    let mut rules = Vec::new();
    let mut rest = text.as_str();
    while let Some(open) = find_outside_strings(rest, b'{') {
        let prelude = rest[..open].trim();
        let Some(close) = block_end(&rest[open..]) else {
            break;
        };
        let body = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        if prelude.starts_with('@') || find_outside_strings(body, b'{').is_some() {
            continue;
        }
        rules.push(Rule {
            selectors: prelude
                .split(',')
                .map(|selector| selector.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|selector| !selector.is_empty())
                .collect(),
            declarations: declarations(body),
        });
    }
    rules
}

fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        rest = rest[start + 2..]
            .find("*/")
            .map_or("", |end| &rest[start + 2 + end + 2..]);
    }
    out.push_str(rest);
    out
}

/// Where `byte` first is, outside quotes.
fn find_outside_strings(text: &str, byte: u8) -> Option<usize> {
    let mut quote = None;
    for (at, &b) in text.as_bytes().iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == byte => return Some(at),
            None => {}
        }
    }
    None
}

/// Where the block opening at the start of `text` closes.
fn block_end(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (at, &b) in text.as_bytes().iter().enumerate() {
        match (quote, b) {
            (Some(q), _) if b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(b),
            (None, b'{') => depth += 1,
            (None, b'}') => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn declarations(body: &str) -> Vec<(String, String)> {
    body.split(';')
        .filter_map(|declaration| declaration.split_once(':'))
        .map(|(name, value)| {
            let value = value.trim();
            let value = value.strip_suffix("!important").unwrap_or(value).trim();
            (name.trim().to_ascii_lowercase(), value.to_owned())
        })
        .filter(|(name, _)| !name.is_empty())
        .collect()
}

/// A selector's device and what it picks within it, from either of GG's
/// two views of a device: `#GameIntegration .device-<name> ...` and
/// `#DeviceConfig.<name> ...`.
fn scoped(selector: &str) -> Option<(&str, &str)> {
    let rest = selector
        .strip_prefix("#GameIntegration .device-")
        .or_else(|| selector.strip_prefix("#DeviceConfig."))?;
    let (device, within) = rest.split_once(' ').unwrap_or((rest, ""));
    let name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    (!device.is_empty() && device.chars().all(name_char)).then_some((device, within))
}

/// CSS pixels, from a length such as `12px` or `12.5px`; others aren't.
fn pixels(value: &str) -> Option<f32> {
    let value = value.trim();
    let number = value.strip_suffix("px").unwrap_or(value);
    if number != value || number == "0" {
        number.trim().parse().ok().filter(|n: &f32| n.is_finite())
    } else {
        None
    }
}

/// The picture a `background` or `background-image` names.
fn url(value: &str) -> Option<String> {
    let (_, rest) = value.split_once("url(")?;
    let (inner, _) = rest.split_once(')')?;
    let inner = inner.trim().trim_matches(|c| c == '"' || c == '\'');
    (!inner.is_empty()).then(|| inner.to_owned())
}

type Declarations = Vec<(String, String)>;

/// The devices with key markers or light zones, and a picture, by name.
pub fn devices(rules: &[Rule]) -> BTreeMap<String, Device> {
    #[derive(Default)]
    struct Found {
        markers: BTreeMap<u16, Declarations>,
        zones: BTreeMap<u16, Declarations>,
        marker_size: (Option<f32>, Option<f32>),
        pictures: BTreeMap<Option<u32>, Picture>,
        container_size: Option<(f32, f32)>,
    }
    let mut found: BTreeMap<String, Found> = BTreeMap::new();
    for rule in rules {
        let get = |name: &str| {
            rule.declarations
                .iter()
                .rev()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        let size = get("background-size").and_then(|value| {
            let mut parts = value.split_whitespace().map(pixels);
            Some((parts.next()??, parts.next()??))
        });
        let contained = (get("background-size") == Some("contain"))
            .then(|| {
                Some((
                    get("width").and_then(pixels)?,
                    get("height").and_then(pixels)?,
                ))
            })
            .flatten();
        for selector in &rule.selectors {
            let Some((name, within)) = scoped(selector) else {
                continue;
            };
            let device = found.entry(name.to_owned()).or_default();
            if let Some(code) = within.strip_prefix(".code")
                && let Ok(code) = code.parse::<u16>()
            {
                device
                    .markers
                    .entry(code)
                    .or_default()
                    .extend(rule.declarations.iter().cloned());
            } else if let Some(code) = within.strip_prefix(".zoneBackgrounds #ZoneColors .code")
                && let Ok(code) = code.parse::<u16>()
            {
                device
                    .zones
                    .entry(code)
                    .or_default()
                    .extend(rule.declarations.iter().cloned());
            } else if within == ".marker" {
                device.marker_size = (
                    get("width").and_then(pixels).or(device.marker_size.0),
                    get("height").and_then(pixels).or(device.marker_size.1),
                );
            } else if let Some(region) = picture_region(within) {
                let picture = get("background-image")
                    .and_then(url)
                    .or_else(|| get("background").and_then(url));
                if let Some(url) = picture {
                    device.pictures.insert(
                        region,
                        Picture {
                            region,
                            url,
                            size,
                            contained,
                        },
                    );
                } else if region.is_none() && size.is_some() {
                    device.container_size = size;
                }
            }
        }
    }
    found
        .into_iter()
        .filter_map(|(name, found)| {
            let (width, height) = found.marker_size;
            let boxes = |found: BTreeMap<u16, Declarations>, size: (Option<f32>, Option<f32>)| {
                found
                    .into_iter()
                    .filter_map(|(code, declarations)| {
                        marker(&declarations, size).map(|m| (code, m))
                    })
                    .collect::<BTreeMap<u16, Marker>>()
            };
            let markers = boxes(
                found.markers,
                (width.or(Some(MARKER_SIZE)), height.or(Some(MARKER_SIZE))),
            );
            let zones = boxes(found.zones, (None, None));
            let pictures: Vec<Picture> = found
                .pictures
                .into_values()
                .map(|picture| Picture {
                    size: picture.size.or(found.container_size),
                    ..picture
                })
                .collect();
            ((!markers.is_empty() || !zones.is_empty()) && !pictures.is_empty()).then_some((
                name,
                Device {
                    markers,
                    zones,
                    pictures,
                },
            ))
        })
        .collect()
}

/// A marker's box from its rules' declarations, the last of each name
/// winning, its size `size` where they don't say; none if hidden.
fn marker(declarations: &Declarations, size: (Option<f32>, Option<f32>)) -> Option<Marker> {
    let get = |key: &str| {
        declarations
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };
    if get("display") == Some("none") {
        return None;
    }
    Some(Marker {
        left: get("left").and_then(pixels)?,
        top: get("top").and_then(pixels)?,
        width: get("width").and_then(pixels).or(size.0)?,
        height: get("height").and_then(pixels).or(size.1)?,
    })
}

/// Whether what a selector picks is the picture's box, and for which
/// region: `.deviceImg.region-3 .deviceImgContainer`, or without a region.
fn picture_region(within: &str) -> Option<Option<u32>> {
    if within == ".deviceImgContainer" {
        return Some(None);
    }
    let image = within
        .strip_suffix(" .deviceImgContainer")
        .unwrap_or(within);
    if image == ".deviceImg" {
        return Some(None);
    }
    image
        .strip_prefix(".deviceImg.region-")?
        .parse()
        .ok()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = r#"
/* GG's, much cut down. */
@font-face { font-family: "x"; src: url(font.woff2) }
@media (max-width: 100px) { #GameIntegration .device-kbd .code41 { left: 900px; top: 900px } }
#GameIntegration .device-kbd .marker, #DeviceConfig.kbd .marker { width: 30px; height: 20px; }
#DeviceConfig.kbd .deviceImg.region-3 .deviceImgContainer,
#GameIntegration .device-kbd .deviceImg.region-3 .deviceImgContainer {
  background: transparent url(us.png) no-repeat; background-size: 730px 250px;
}
#GameIntegration .device-kbd .deviceImg.region-1 .deviceImgContainer { background-image: url("jp.png") }
#GameIntegration .device-kbd .deviceImgContainer { background-size: 700px 200px }
#GameIntegration .device-kbd .code41 { left: 10px; top: 12px }
#GameIntegration .device-kbd .code40 { left: 500px; top: 62px; width: 57px !important; height: 78px }
#GameIntegration .device-kbd .code58 { left: 50px; top: 12px; display: none }
#GameIntegration .device-kbd .region-1 .code41 { left: 1px; top: 1px }
#GameIntegration .device-kbd .code41 > a:after { content: '{'; left: -15px }
#GameIntegration .device-mouse .code1 { left: 1px; top: 1px }
.something .else { color: red }
"#;

    #[test]
    fn reads_the_rules() {
        let rules = rules(SHEET);
        assert_eq!(
            rules
                .iter()
                .filter(|rule| rule.selectors.len() == 2)
                .count(),
            2
        );
        let marker = &rules[0];
        assert_eq!(
            marker.selectors,
            [
                "#GameIntegration .device-kbd .marker",
                "#DeviceConfig.kbd .marker"
            ]
        );
        assert_eq!(marker.declarations[1], ("height".into(), "20px".into()));
        let quoted = rules
            .iter()
            .find(|rule| rule.selectors[0].ends_with("a:after"))
            .unwrap();
        assert_eq!(quoted.declarations[0], ("content".into(), "'{'".into()));
        assert!(rules.iter().all(|rule| !rule.selectors[0].starts_with('@')));
    }

    #[test]
    fn finds_the_markers_and_pictures() {
        let devices = devices(&rules(SHEET));
        assert_eq!(devices.keys().collect::<Vec<_>>(), ["kbd"]);
        let kbd = &devices["kbd"];
        assert_eq!(
            kbd.markers[&41],
            Marker {
                left: 10.0,
                top: 12.0,
                width: 30.0,
                height: 20.0
            }
        );
        assert_eq!(kbd.markers[&40].centre(), (528.5, 101.0));
        assert!(!kbd.markers.contains_key(&58));
        assert_eq!(
            kbd.pictures,
            [
                Picture {
                    region: Some(1),
                    url: "jp.png".into(),
                    size: Some((700.0, 200.0)),
                    contained: None,
                },
                Picture {
                    region: Some(3),
                    url: "us.png".into(),
                    size: Some((730.0, 250.0)),
                    contained: None,
                },
            ]
        );
    }

    #[test]
    fn finds_the_zones_of_lights_beside() {
        let sheet = "
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code0,
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code1 { width: 129px; height: 16.2px }
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code0 { left: 539px; top: 162px }
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code1 { left: 539px; top: 178.2px }
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code5 { top: 53px; left: 172px; width: 50px; height: 56px }
#GameIntegration .device-alc .zoneBackgrounds #ZoneColors .code5 { top: 93px; left: 310px; width: 45px; height: 54px }
#GameIntegration .device-alc .deviceImg .deviceImgContainer { background: url(lid.png); width: 800px; height: 265px; background-size: contain }
";
        let devices = devices(&rules(sheet));
        let alc = &devices["alc"];
        assert!(alc.markers.is_empty());
        assert_eq!(alc.zones.len(), 3);
        assert_eq!(alc.zones[&1].top, 178.2);
        // The later rule wins, as in a browser.
        assert_eq!(
            alc.zones[&5],
            Marker {
                left: 310.0,
                top: 93.0,
                width: 45.0,
                height: 54.0
            }
        );
        let picture = &alc.pictures[0];
        assert_eq!(picture.contained, Some((800.0, 265.0)));
        assert_eq!(picture.drawn(1600, 525), Some((800.0, 262.5)));
    }

    #[test]
    fn reads_lengths() {
        assert_eq!(pixels("12px"), Some(12.0));
        assert_eq!(pixels(" 16.2px"), Some(16.2));
        assert_eq!(pixels("0"), Some(0.0));
        assert_eq!(pixels("50%"), None);
        assert_eq!(pixels("auto"), None);
        assert_eq!(
            url("transparent url( 'a/b.png' ) no-repeat"),
            Some("a/b.png".into())
        );
    }
}
